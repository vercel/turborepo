//! Makefiles (`GNUmakefile`, `makefile`, `Makefile`, and the `*.mk` files they
//! include): static rules, pattern rules, `$(wildcard)`, recursive `make -C`.
//!
//! See [`crate::sources`] for the [`TaskSource`]
//! contract and [`mod@crate::registry`] for the [`Recognizer`] contract.
//!
//! # Model
//!
//! [`parse`] reads a makefile into variables and rules ([`eval`] expands make
//! syntax, never executing anything). [`analyze`] turns the rules into targets:
//! explicit targets (rules merged per target, double-colon recipes unioned,
//! static pattern rules split per target), plus the targets GNU make would
//! build through pattern rules (user pattern rules, then the built-in C/C++/
//! assembler rules) for prerequisites that have no explicit rule.
//!
//! Every target is a task `make:<dir>:<target>`:
//!
//! | Make | Task |
//! |---|---|
//! | the makefile and every included file | configs (missing includes are probes) |
//! | prerequisites (normal and order-only) that are targets | task deps ([`TaskRef::Id`]) |
//! | other file prerequisites | inputs (a missing one is a probe) |
//! | `$(wildcard …)` and wildcard prerequisites | memberships (adding a matching file changes the task) |
//! | recipe lines (automatic and target-specific variables expanded) | command lines, through the shell parser and the registry |
//! | a non-phony file target (a name with `/` or `.`) with a recipe | an output (generated-file linking) |
//! | unknown parts (`$(eval)`, unknown conditionals, computed prerequisites, `vpath`) | uncertainties, plus a membership on the makefile's directory |
//!
//! # Discovery
//!
//! One makefile per directory (GNU make's order: `GNUmakefile`, `makefile`,
//! `Makefile`), up to 8 levels below each package root. Directory listings made
//! by the walk and by `$(wildcard)` are not recorded as observations shared by
//! every task of the package (the store would link each task to any producer
//! whose output lies in a listed directory); instead the walk records one
//! listing (`**/{GNUmakefile,makefile,Makefile}`) and `$(wildcard)` globs
//! become memberships of the tasks that use them. [`MakeRun`] (`make -C
//! dir target`, `$(MAKE) target`) links to `make:` tasks, and falls back to a
//! membership of the directory for makefiles the source does not discover (`-f
//! other.mk`, `build/`).
//!
//! Recipe words containing unknown values (`$(shell …)`, variables only the
//! environment defines) reach the shell parser as unset variables, so the
//! registry treats them as computed arguments, except after `=` or `:` in the
//! same word (`-X main.version=$(VERSION)`, `image:$(TAG)`), which is treated
//! as a literal (flag values and image tags, not paths).

pub mod cc;
pub mod eval;
pub mod parse;

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use turborepo_trace_core::UncertaintyKind;
use turborepo_trace_frameworks::fsx;

use self::{
    eval::{Exp, Glob, Issue, Origin, Var, Vars, has_unknown, pattern_match, subst_stem},
    parse::{Db, push_issue},
};
use crate::{
    registry::{Recognition, RecognizeCx, Recognizer},
    sources::{DiscoveredTask, SourceCx, TaskSource, Workspace},
    spec::{Command, OutputKind, TaskId, TaskRef, TaskSpec},
};

/// Task sources of this module.
pub fn sources() -> Vec<&'static dyn TaskSource> {
    vec![&MakeSource]
}

/// Recognizers of this module (registered before the generic fallbacks).
pub fn recognizers() -> Vec<&'static dyn Recognizer> {
    vec![&MakeRun, &cc::Cc, &cc::Ar]
}

/// Default makefile names, in GNU make's lookup order.
pub const MAKEFILE_NAMES: &[&str] = &["GNUmakefile", "makefile", "Makefile"];

/// How deep below a package root makefiles are looked for.
const MAX_DEPTH: usize = 8;
/// Maximum number of targets per makefile.
const MAX_TARGETS: usize = 5_000;

/// The makefile GNU make would read in `dir`.
pub fn find_makefile(dir: &Path) -> Option<PathBuf> {
    fsx::first_existing(dir, MAKEFILE_NAMES)
}

/// One recipe of a target, with what its automatic variables refer to.
#[derive(Clone, Debug, Default)]
pub struct Recipe {
    /// Raw lines.
    pub lines: Vec<String>,
    /// `$*`.
    pub stem: String,
    /// The first prerequisite of the rule that provided the recipe (`$<`), if
    /// any.
    pub first: Option<String>,
}

/// A target to build.
#[derive(Clone, Debug, Default)]
pub struct Target {
    /// Name.
    pub name: String,
    /// Normal prerequisites (in order, deduplicated).
    pub prereqs: Vec<String>,
    /// Order-only prerequisites.
    pub order_only: Vec<String>,
    /// Recipes (one, or several for double-colon rules).
    pub recipes: Vec<Recipe>,
    /// Issues of the rules that define it.
    pub issues: Vec<Issue>,
    /// Globs listed by its rules.
    pub globs: Vec<Glob>,
    /// Built through a pattern rule only.
    pub implicit: bool,
    /// Declared `.PHONY`.
    pub phony: bool,
}

/// A makefile with its targets resolved.
#[derive(Clone, Debug, Default)]
pub struct Analysis {
    /// The parsed makefile.
    pub db: Db,
    /// Targets by name.
    pub targets: BTreeMap<String, Target>,
}

/// A pattern rule usable for implicit targets.
#[derive(Clone, Debug)]
struct PatternRule {
    targets: Vec<String>,
    prereqs: Vec<String>,
    order_only: Vec<String>,
    recipe: Vec<String>,
    issues: Vec<Issue>,
}

/// GNU make's built-in pattern rules that matter for builds (C, C++, assembler,
/// linking).
const BUILTIN_RULES: &[(&str, &str, &str)] = &[
    ("%.o", "%.c", "$(COMPILE.c) $(OUTPUT_OPTION) $<"),
    ("%.o", "%.cc", "$(COMPILE.cc) $(OUTPUT_OPTION) $<"),
    ("%.o", "%.cpp", "$(COMPILE.cpp) $(OUTPUT_OPTION) $<"),
    ("%.o", "%.cxx", "$(COMPILE.cc) $(OUTPUT_OPTION) $<"),
    ("%.o", "%.C", "$(COMPILE.C) $(OUTPUT_OPTION) $<"),
    ("%.o", "%.s", "$(AS) $(ASFLAGS) -o $@ $<"),
    (
        "%.o",
        "%.S",
        "$(CC) $(ASFLAGS) $(CPPFLAGS) $(TARGET_ARCH) -c -o $@ $<",
    ),
    ("%", "%.o", "$(LINK.o) $^ $(LOADLIBES) $(LDLIBS) -o $@"),
    ("%", "%.c", "$(LINK.c) $^ $(LOADLIBES) $(LDLIBS) -o $@"),
];

/// Parses `makefile` and resolves its targets.
pub fn analyze(makefile: &Path) -> Analysis {
    let db = parse::parse_file(makefile);
    resolve(db)
}

/// Resolves the targets of a parsed makefile.
// Invariant: `n` is a key of `a.targets`.
#[allow(clippy::expect_used)]
pub fn resolve(db: Db) -> Analysis {
    let mut targets: BTreeMap<String, Target> = BTreeMap::new();
    for r in db.rules.iter().filter(|r| !r.pattern) {
        for t in &r.targets {
            if parse::is_special_target(t) || t.is_empty() {
                continue;
            }
            if targets.len() >= MAX_TARGETS && !targets.contains_key(t) {
                break;
            }
            let e = targets.entry(t.clone()).or_insert_with(|| Target {
                name: t.clone(),
                phony: db.phony.contains(t),
                ..Default::default()
            });
            extend_dedup(&mut e.prereqs, &r.prereqs);
            extend_dedup(&mut e.order_only, &r.order_only);
            for i in &r.issues {
                push_issue(&mut e.issues, i.clone());
            }
            for g in &r.globs {
                if !e.globs.contains(g) {
                    e.globs.push(g.clone());
                }
            }
            if !r.recipe.is_empty() {
                let rec = Recipe {
                    lines: r.recipe.clone(),
                    stem: r.stem.clone().unwrap_or_else(|| explicit_stem(&db, t)),
                    first: r.prereqs.first().cloned(),
                };
                if r.double_colon {
                    e.recipes.push(rec);
                } else {
                    // The last recipe wins (make warns).
                    e.recipes = vec![rec];
                }
            }
        }
    }
    let mut a = Analysis { db, targets };
    let rules = a.pattern_rules();
    let mentions: BTreeSet<String> = a
        .targets
        .values()
        .flat_map(|t| {
            std::iter::once(t.name.clone())
                .chain(t.prereqs.iter().cloned())
                .chain(t.order_only.iter().cloned())
        })
        .collect();
    // Explicit targets without a recipe get one from a pattern rule.
    let names: Vec<String> = a.targets.keys().cloned().collect();
    for n in &names {
        let t = &a.targets[n];
        if !t.recipes.is_empty() || t.phony {
            continue;
        }
        if let Some(m) = a.implicit(&rules, &mentions, n, 0) {
            let t = a.targets.get_mut(n).expect("target");
            extend_dedup(&mut t.prereqs, &m.prereqs);
            extend_dedup(&mut t.order_only, &m.order_only);
            t.recipes.push(m.recipe);
            for i in m.issues {
                push_issue(&mut t.issues, i);
            }
        }
    }
    // Prerequisites built through pattern rules become (implicit) targets.
    let mut queue: Vec<String> = a
        .targets
        .values()
        .flat_map(|t| t.prereqs.iter().chain(&t.order_only).cloned())
        .collect();
    while let Some(p) = queue.pop() {
        if a.targets.len() >= MAX_TARGETS {
            break;
        }
        if a.targets.contains_key(&p) || a.db.phony.contains(&p) {
            continue;
        }
        if let Some(m) = a.implicit(&rules, &mentions, &p, 0) {
            queue.extend(m.prereqs.iter().chain(&m.order_only).cloned());
            a.targets.insert(
                p.clone(),
                Target {
                    name: p.clone(),
                    prereqs: m.prereqs,
                    order_only: m.order_only,
                    recipes: vec![m.recipe],
                    issues: m.issues,
                    globs: Vec::new(),
                    implicit: true,
                    phony: false,
                },
            );
        }
    }
    a
}

/// `$*` of an explicit rule: the target without a known suffix.
fn explicit_stem(db: &Db, t: &str) -> String {
    let base_start = t.rfind('/').map_or(0, |i| i + 1);
    match t[base_start..].rfind('.') {
        Some(i) if db.suffixes.iter().any(|s| *s == t[base_start + i..]) => {
            t[..base_start + i].to_string()
        }
        _ => String::new(),
    }
}

fn extend_dedup(v: &mut Vec<String>, add: &[String]) {
    for a in add {
        if !v.contains(a) {
            v.push(a.clone());
        }
    }
}

/// A pattern rule applied to one file.
struct Implicit {
    prereqs: Vec<String>,
    order_only: Vec<String>,
    recipe: Recipe,
    issues: Vec<Issue>,
}

impl Analysis {
    /// User pattern rules with a recipe, then the built-in ones they don't
    /// cancel.
    fn pattern_rules(&self) -> Vec<PatternRule> {
        let mut out = Vec::new();
        let mut cancelled: BTreeSet<(String, String)> = BTreeSet::new();
        for r in self.db.rules.iter().filter(|r| r.pattern) {
            if r.recipe.is_empty() {
                for t in &r.targets {
                    cancelled.insert((t.clone(), r.prereqs.join(" ")));
                }
                continue;
            }
            out.push(PatternRule {
                targets: r.targets.clone(),
                prereqs: r.prereqs.clone(),
                order_only: r.order_only.clone(),
                recipe: r.recipe.clone(),
                issues: r.issues.clone(),
            });
        }
        let no_builtins = self.db.vars.get("MAKEFLAGS").is_some_and(|v| {
            eval::words(&v.value).any(|w| {
                w == "--no-builtin-rules"
                    || (w.starts_with('-') && !w.starts_with("--") && w.contains('r'))
            })
        });
        if !no_builtins {
            for (t, p, recipe) in BUILTIN_RULES {
                if cancelled.contains(&(t.to_string(), p.to_string())) {
                    continue;
                }
                out.push(PatternRule {
                    targets: vec![t.to_string()],
                    prereqs: vec![p.to_string()],
                    order_only: Vec::new(),
                    recipe: vec![recipe.to_string()],
                    issues: Vec::new(),
                });
            }
        }
        out
    }

    /// The pattern rule GNU make would use for `file`: every prerequisite
    /// exists, is mentioned in the makefile, or can itself be made by a
    /// pattern rule (bounded chain).
    fn implicit(
        &self,
        rules: &[PatternRule],
        mentions: &BTreeSet<String>,
        file: &str,
        depth: usize,
    ) -> Option<Implicit> {
        if file.contains('%') || file.is_empty() || depth > 2 || has_unknown(file) {
            return None;
        }
        for r in rules {
            for tp in &r.targets {
                // Match-anything rules only for direct targets.
                if tp == "%" && depth > 0 {
                    continue;
                }
                let Some((stem, dirpart, base_stem)) = match_target(tp, file) else {
                    continue;
                };
                let sub = |p: &String| -> String {
                    if !p.contains('%') {
                        p.clone()
                    } else if dirpart.is_empty() || p.contains('/') {
                        subst_stem(p, &stem)
                    } else {
                        format!("{dirpart}{}", subst_stem(p, &base_stem))
                    }
                };
                let prereqs: Vec<String> = r.prereqs.iter().map(sub).collect();
                if prereqs.iter().any(|p| p == file) {
                    continue;
                }
                let ok = prereqs.iter().all(|p| {
                    mentions.contains(p)
                        || self.targets.contains_key(p)
                        || fsx::exists(&fsx::join(&self.db.dir, p))
                        || self.implicit(rules, mentions, p, depth + 1).is_some()
                });
                if !ok {
                    continue;
                }
                return Some(Implicit {
                    order_only: r.order_only.iter().map(sub).collect(),
                    recipe: Recipe {
                        lines: r.recipe.clone(),
                        stem,
                        first: prereqs.first().cloned(),
                    },
                    prereqs,
                    issues: r.issues.clone(),
                });
            }
        }
        None
    }

    /// Whether `target` is something this makefile knows how to build.
    pub fn has_target(&self, target: &str) -> bool {
        if self.targets.contains_key(target) {
            return true;
        }
        let rules = self.pattern_rules();
        let mentions = BTreeSet::new();
        self.implicit(&rules, &mentions, target, 0).is_some()
    }

    /// Variables exported to recipes whose value is known.
    pub fn exported_env(&self) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        for (k, v) in &self.db.vars {
            if !(v.exported || self.db.export_all && v.origin == Origin::File) {
                continue;
            }
            if !k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                continue;
            }
            let mut e = Exp::new(&self.db.vars, &self.db.dir);
            let val = e.var(k);
            if !has_unknown(&val) && e.issues.is_empty() {
                out.insert(k.clone(), val);
            }
        }
        out
    }

    /// The expanded recipe command lines of `t`, with their issues and globs.
    pub fn recipe_lines(&self, t: &Target) -> (Vec<String>, Vec<Issue>, Vec<Glob>) {
        let mut lines = Vec::new();
        let mut issues = Vec::new();
        let mut globs = Vec::new();
        let tvars = self.target_vars(&t.name, &mut issues, &mut globs);
        for r in &t.recipes {
            let mut e = Exp::new(&self.db.vars, &self.db.dir);
            e.push_locals(tvars.clone());
            let mut auto = Vars::new();
            let mut set = |k: &str, v: String| {
                auto.insert(
                    k.to_string(),
                    Var {
                        value: v,
                        origin: Origin::Automatic,
                        ..Default::default()
                    },
                );
            };
            set("@", t.name.clone());
            set(
                "<",
                r.first
                    .clone()
                    .or_else(|| t.prereqs.first().cloned())
                    .unwrap_or_default(),
            );
            set("^", t.prereqs.join(" "));
            set("+", t.prereqs.join(" "));
            set("?", t.prereqs.join(" "));
            set("*", r.stem.clone());
            set("|", t.order_only.join(" "));
            set("%", String::new());
            e.push_locals(auto);
            let mut cmds = Vec::new();
            for raw in &r.lines {
                let x = e.expand(raw);
                for piece in split_recipe(&x) {
                    let p = strip_recipe_prefixes(&piece);
                    if !p.trim().is_empty() {
                        cmds.push(recipe_text(p));
                    }
                }
            }
            if self.db.oneshell && !cmds.is_empty() {
                lines.push(cmds.join("\n"));
            } else {
                lines.extend(cmds);
            }
            for i in e.issues {
                push_issue(&mut issues, i);
            }
            for g in e.globs {
                if !globs.contains(&g) {
                    globs.push(g);
                }
            }
        }
        (lines, issues, globs)
    }

    /// The target-specific (and pattern-specific) variables of `target`.
    fn target_vars(&self, target: &str, issues: &mut Vec<Issue>, globs: &mut Vec<Glob>) -> Vars {
        let mut out = Vars::new();
        for tv in &self.db.target_vars {
            let applies = tv.target == target
                || (tv.target.contains('%') && pattern_match(&tv.target, target).is_some());
            if !applies {
                continue;
            }
            for i in &tv.issues {
                push_issue(issues, i.clone());
            }
            let existing = out
                .get(&tv.name)
                .cloned()
                .or_else(|| self.db.vars.get(&tv.name).cloned());
            let mut e = Exp::new(&self.db.vars, &self.db.dir);
            e.push_locals(out.clone());
            let var = match tv.op.as_str() {
                ":=" | "::=" | ":::=" => Var::simple(e.expand(&tv.rhs)),
                "?=" => match existing {
                    Some(_) => continue,
                    None => Var::recursive(tv.rhs.clone()),
                },
                "+=" => match existing {
                    Some(old) if !old.recursive => {
                        Var::simple(format!("{} {}", old.value, e.expand(&tv.rhs)))
                    }
                    Some(old) => Var::recursive(format!("{} {}", old.value, tv.rhs)),
                    None => Var::recursive(tv.rhs.clone()),
                },
                "!=" => Var::simple(eval::unknown("shell")),
                _ => Var::recursive(tv.rhs.clone()),
            };
            for i in e.issues {
                push_issue(issues, i);
            }
            for g in e.globs {
                if !globs.contains(&g) {
                    globs.push(g);
                }
            }
            out.insert(tv.name.clone(), var);
        }
        out
    }

    /// The task for target `name`.
    pub fn task(&self, ws: &Workspace, name: &str) -> Option<DiscoveredTask> {
        let t = self.targets.get(name)?;
        let dir = &self.db.dir;
        let rel = ws.rel(dir);
        let mut d = DiscoveredTask::new(TaskId::make(rel.clone(), name), name, dir.clone());
        d.files = self.db.files.clone();
        for m in &self.db.missing {
            d.spec.config(m.clone());
        }
        for (p, order_only) in t
            .prereqs
            .iter()
            .map(|p| (p, false))
            .chain(t.order_only.iter().map(|p| (p, true)))
        {
            if self.targets.contains_key(p) {
                d.spec.dep(
                    TaskRef::Id {
                        id: TaskId::make(rel.clone(), p.clone()),
                    },
                    if order_only {
                        "make order-only prerequisite"
                    } else {
                        "make prerequisite"
                    },
                );
                continue;
            }
            if self.db.phony.contains(p) || order_only {
                // An order-only file (`| $(BUILD_DIR)`): only its existence matters.
                continue;
            }
            // A file or directory (no filesystem check: see outputs below).
            d.spec.input(fsx::join(dir, p));
        }
        // A file target is an output. Bare names (`test`, `build`, `app`) are usually
        // phony or directories (`test/` would cover sources); their recipes'
        // recognizers report what they write (`cc -o app`). No filesystem check
        // here: a probe in task discovery would be shared by every task of the
        // package and link them all to this producer.
        if !t.phony && !t.recipes.is_empty() && name.contains(['/', '.']) {
            d.spec
                .output_of_kind(fsx::join(dir, name), OutputKind::Declared);
        }
        let (lines, rissues, rglobs) = self.recipe_lines(t);
        d.lines = lines;
        d.env = self.exported_env();
        let mut issues = t.issues.clone();
        for i in rissues.into_iter().chain(self.db.issues.iter().cloned()) {
            push_issue(&mut issues, i);
        }
        for g in t.globs.iter().chain(&rglobs).chain(&self.db.globs) {
            d.spec
                .membership(g.dir.clone(), g.pattern.clone(), "make $(wildcard)");
        }
        if !issues.is_empty() {
            for (k, detail) in issues {
                d.spec.uncertain(k, detail);
            }
            d.spec
                .membership(dir.clone(), "**", "makefile not fully understood");
        }
        Some(d)
    }
}

/// Matches a target pattern: (stem for `$*`, directory part, stem without the
/// directory). A pattern without `/` matches the file name, the directory going
/// into the stem.
fn match_target(tp: &str, f: &str) -> Option<(String, String, String)> {
    if !tp.contains('/')
        && let Some(i) = f.rfind('/')
    {
        let (dirpart, base) = (&f[..=i], &f[i + 1..]);
        let s = pattern_match(tp, base)?;
        if s.is_empty() && tp.contains('%') {
            return None;
        }
        return Some((format!("{dirpart}{s}"), dirpart.to_string(), s.to_string()));
    }
    let s = pattern_match(tp, f)?;
    if s.is_empty() && tp.contains('%') {
        return None;
    }
    Some((s.to_string(), String::new(), s.to_string()))
}

/// Splits an expanded recipe line at newlines that are not continuations (a
/// multi-line variable expands to several recipe lines).
fn split_recipe(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for (i, part) in s.split('\n').enumerate() {
        if i > 0 {
            if cur.ends_with('\\') {
                cur.push('\n');
            } else {
                out.push(std::mem::take(&mut cur));
            }
        }
        cur.push_str(part);
    }
    out.push(cur);
    out
}

/// Removes leading whitespace and the `@`, `-`, `+` recipe prefixes.
fn strip_recipe_prefixes(s: &str) -> &str {
    let mut t = s.trim_start();
    while let Some(r) = t
        .strip_prefix('@')
        .or_else(|| t.strip_prefix('-'))
        .or_else(|| t.strip_prefix('+'))
    {
        t = r.trim_start();
    }
    t
}

/// Unknown markers to shell text: a literal placeholder after `=` or `:` in the
/// same word (`-X main.version=$(VERSION)`, `app:$(TAG)`), else an unset
/// variable (a dynamic word).
fn recipe_text(s: &str) -> String {
    if !has_unknown(s) {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 16);
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == eval::UNK_OPEN {
            let mut name = String::new();
            for d in it.by_ref() {
                if d == eval::UNK_CLOSE {
                    break;
                }
                name.push(d);
            }
            let word_start = out.rfind([' ', '\t', '\'', '"']).map_or(0, |i| i + 1);
            if out[word_start..].contains(['=', ':']) {
                out.push_str(&format!("__make_{name}__"));
            } else {
                out.push_str(&format!("${{__make_{name}}}"));
            }
        } else if c != eval::UNK_CLOSE {
            out.push(c);
        }
    }
    out
}

/// `Makefile` task source.
pub struct MakeSource;

impl TaskSource for MakeSource {
    fn id(&self) -> &'static str {
        "make"
    }

    fn config_file_names(&self) -> &'static [&'static str] {
        MAKEFILE_NAMES
    }

    fn discover(&self, cx: &SourceCx<'_>) -> Vec<DiscoveredTask> {
        let found = find_quietly(
            cx,
            MAX_DEPTH,
            &|n| MAKEFILE_NAMES.contains(&n),
            "**/{GNUmakefile,makefile,Makefile}",
        );
        // One makefile per directory, in GNU make's preference order.
        let mut by_dir: BTreeMap<PathBuf, PathBuf> = BTreeMap::new();
        for f in found {
            let Some(dir) = f.parent() else { continue };
            let rank = |p: &Path| {
                let n = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                MAKEFILE_NAMES.iter().position(|m| *m == n).unwrap_or(9)
            };
            match by_dir.get(dir) {
                Some(old) if rank(old) <= rank(&f) => {}
                _ => {
                    by_dir.insert(dir.to_path_buf(), f.clone());
                }
            }
        }
        let mut out = Vec::new();
        for mk in by_dir.values() {
            // Directory listings made while evaluating the makefile (`$(wildcard)`) become
            // memberships of the tasks that use them instead of observations shared by
            // every task of the package.
            turborepo_trace_core::record::pause_listings(|| {
                let a = analyze(mk);
                for name in a.targets.keys() {
                    if let Some(t) = a.task(cx.workspace, name) {
                        out.push(t);
                    }
                }
            });
        }
        out
    }
}

/// [`SourceCx::find_files`] without recording its directory listings, which
/// would be observations shared by every task of the package (the store links a
/// task to the producer of any output inside a directory it listed). Records
/// one narrow listing instead (`listing`, a glob relative to the package
/// directory), so adding a matching file still re-discovers the package's
/// tasks.
pub(crate) fn find_quietly(
    cx: &SourceCx<'_>,
    max_depth: usize,
    pred: &dyn Fn(&str) -> bool,
    listing: &str,
) -> Vec<PathBuf> {
    use std::sync::{Arc, OnceLock};

    use turborepo_trace_core::record;
    // Every source of the package walks the same tree (the root package's is the
    // whole repository): walk once per shared dir cache, package and depth,
    // keeping every file, then filter by name. `pred` only selects files, never
    // which directories are entered. Which directories the walk enters depends
    // on the packages nested below this one (`SourceCx::owns`): their roots are
    // part of the key. Packages are sorted by `rel`, so they form one
    // contiguous block.
    let nested = {
        use std::hash::{Hash, Hasher};
        let pkgs = &cx.workspace.packages;
        let rel = cx.package.rel.as_str();
        let block: &[crate::sources::Package] = if rel == "." {
            pkgs
        } else {
            let prefix = format!("{rel}/");
            let lo = pkgs.partition_point(|p| p.rel.as_str() < prefix.as_str());
            let n = pkgs[lo..]
                .iter()
                .take_while(|p| p.rel.starts_with(prefix.as_str()))
                .count();
            &pkgs[lo..lo + n]
        };
        let mut h = xxhash_rust::xxh3::Xxh3::new();
        for p in block {
            p.dir.hash(&mut h);
        }
        (block.len(), h.finish())
    };
    type Key = (PathBuf, PathBuf, usize, (usize, u64));
    static M: OnceLock<crate::recognizers::bins::RecMemo<Key, Arc<Vec<PathBuf>>>> = OnceLock::new();
    let all = M
        .get_or_init(crate::recognizers::bins::RecMemo::new)
        .get_or(
            (
                cx.workspace.root.clone(),
                cx.package.dir.clone(),
                max_depth,
                nested,
            ),
            || {
                Arc::new(record::pause_listings(|| {
                    cx.find_files(max_depth, &|_| true)
                }))
            },
        );
    let found = all
        .iter()
        .filter(|p| pred(p.file_name().and_then(|n| n.to_str()).unwrap_or("")))
        .cloned()
        .collect();
    record::listing(&cx.package.dir, listing);
    found
}

/// `make [-C dir] [-f file] [targets]`, `gmake`, `$(MAKE)`: a dep on
/// `make:<dir>:<target>` (the default goal without targets).
pub struct MakeRun;

impl Recognizer for MakeRun {
    fn id(&self) -> &'static str {
        "make"
    }

    fn programs(&self) -> &'static [&'static str] {
        &["make", "gmake", "remake"]
    }

    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        out.tool("system", "make");
        if cmd.arg_words().iter().any(|w| w.dynamic) {
            // `make -C $$d` in a loop: any directory, any target.
            out.uncertain(
                UncertaintyKind::UnknownCommand,
                "make with computed arguments",
            );
            cx.whole_package(out, "make with computed arguments");
            return Recognition::Partial;
        }
        let mut dir = cmd.cwd.clone();
        let mut file: Option<String> = None;
        let mut targets = Vec::new();
        let words: Vec<&str> = cmd.args().collect();
        let mut i = 0;
        while i < words.len() {
            let w = words[i];
            i += 1;
            let take = |i: &mut usize| -> Option<String> {
                let v = words.get(*i).map(|s| s.to_string());
                *i += 1;
                v
            };
            match w {
                "-C" | "--directory" => {
                    if let Some(d) = take(&mut i) {
                        dir = fsx::join(&dir, &d);
                    }
                }
                "-f" | "--file" | "--makefile" => file = take(&mut i),
                "-o" | "-W" | "-I" | "--old-file" | "--assume-old" | "--what-if" | "--new-file"
                | "--assume-new" | "--include-dir" | "-O" => {
                    let _ = take(&mut i);
                }
                "-j" | "--jobs" | "-l" | "--load-average" | "--max-load" => {
                    if words.get(i).is_some_and(|n| n.parse::<f64>().is_ok()) {
                        i += 1;
                    }
                }
                "-n" | "--dry-run" | "--just-print" | "--recon" | "-q" | "--question" | "-p"
                | "--print-data-base" | "-v" | "--version" | "-h" | "--help" => {
                    return Recognition::Ignored;
                }
                _ if w.starts_with("--directory=") => {
                    dir = fsx::join(&dir, &w["--directory=".len()..]);
                }
                _ if w.starts_with("-C") && w.len() > 2 => dir = fsx::join(&dir, &w[2..]),
                _ if w.starts_with("--file=") || w.starts_with("--makefile=") => {
                    file = w.split_once('=').map(|(_, v)| v.to_string());
                }
                _ if w.starts_with("-f") && w.len() > 2 && !w.starts_with("--") => {
                    file = Some(w[2..].to_string());
                }
                _ if w.starts_with('-') => {}
                // `VAR=value` overrides.
                _ if w.contains('=') => {}
                _ => targets.push(w.to_string()),
            }
        }
        let default_mk = find_makefile(&dir);
        let mk = match &file {
            Some(f) => Some(fsx::join(&dir, f)),
            None => default_mk.clone(),
        };
        let Some(mk) = mk.filter(|m| fsx::is_file(m)) else {
            out.uncertain(
                UncertaintyKind::UnknownCommand,
                format!("make: no makefile in {}", rel_of(cx.repo_root, &dir)),
            );
            out.membership(dir, "**", "make without a makefile");
            return Recognition::Partial;
        };
        if default_mk.as_deref() != Some(mk.as_path()) {
            // Tasks exist only for the default makefile of a directory.
            out.config(mk.clone());
            out.uncertain(
                UncertaintyKind::UnknownCommand,
                format!("make -f {}", rel_of(cx.repo_root, &mk)),
            );
            out.membership(dir, "**", "make with a non-default makefile");
            return Recognition::Partial;
        }
        if !discoverable(cx.repo_root, &mk) {
            // No `make:` tasks exist for it (outside the repo, too deep, or in a skipped
            // directory such as `build/`).
            out.config(mk.clone());
            out.uncertain(
                UncertaintyKind::UnknownCommand,
                format!(
                    "make in {} (makefile not discovered)",
                    rel_of(cx.repo_root, &dir)
                ),
            );
            out.membership(dir, "**", "make in an undiscovered directory");
            return Recognition::Partial;
        }
        let a = analyze(&mk);
        if targets.is_empty() {
            match a.db.default_goal() {
                Some(g) => targets.push(g),
                None => {
                    out.uncertain(
                        UncertaintyKind::UnknownCommand,
                        format!("make: no default goal in {}", rel_of(cx.repo_root, &mk)),
                    );
                    out.membership(dir, "**", "make without a default goal");
                    return Recognition::Partial;
                }
            }
        }
        let rel = rel_of(cx.repo_root, &dir);
        let mut partial = false;
        for t in targets {
            if a.targets.contains_key(&t) {
                out.dep(
                    TaskRef::Id {
                        id: TaskId::make(rel.clone(), t),
                    },
                    "make",
                );
            } else {
                partial = true;
                out.uncertain(
                    UncertaintyKind::UnknownCommand,
                    format!(
                        "make target `{t}` not found in {}",
                        rel_of(cx.repo_root, &mk)
                    ),
                );
                out.membership(dir.clone(), "**", "unknown make target");
            }
        }
        if partial {
            Recognition::Partial
        } else {
            Recognition::Full
        }
    }
}

/// Whether [`MakeSource`] discovers makefile `mk` (inside the repo, at most
/// [`MAX_DEPTH`] directories down, not in a skipped directory), so `make:` ids
/// for it resolve.
fn discoverable(repo_root: &Path, mk: &Path) -> bool {
    let Ok(rel) = mk.strip_prefix(repo_root) else {
        return false;
    };
    let dirs: Vec<String> = rel
        .parent()
        .map(|p| {
            p.components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    dirs.len() <= MAX_DEPTH
        && dirs
            .iter()
            .all(|d| !fsx::skip_dir(d) && !matches!(d.as_str(), "dist" | "build" | "target"))
}

/// `p` relative to `root`, `/`-separated (`.` for the root itself).
pub fn rel_of(root: &Path, p: &Path) -> String {
    match p.strip_prefix(root) {
        Ok(r) if r.as_os_str().is_empty() => ".".into(),
        Ok(r) => r.to_string_lossy().replace('\\', "/"),
        Err(_) => p.to_string_lossy().into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn an(src: &str) -> Analysis {
        resolve(parse::parse_str(Path::new("/nonexistent-mk/Makefile"), src))
    }

    #[test]
    fn pattern_rules_apply_to_mentioned_files() {
        let a = an(
            "app: main.o util.o\n\t$(CC) -o $@ $^\n%.o: %.c\n\t$(CC) -c $< -o $@\nmain.c util.c:\n",
        );
        let t = &a.targets["main.o"];
        assert!(t.implicit);
        assert_eq!(t.prereqs, ["main.c"]);
        let (lines, issues, _) = a.recipe_lines(t);
        assert_eq!(lines, ["cc -c main.c -o main.o"]);
        assert!(issues.is_empty());
        let (lines, _, _) = a.recipe_lines(&a.targets["app"]);
        assert_eq!(lines, ["cc -o app main.o util.o"]);
    }

    #[test]
    fn builtin_rules_and_dirs() {
        let a = an("bin/app: src/a.o\n\tcc -o $@ $^\nsrc/a.c:\n");
        let t = &a.targets["src/a.o"];
        assert_eq!(t.prereqs, ["src/a.c"]);
        let (lines, _, _) = a.recipe_lines(t);
        let norm: Vec<String> = lines
            .iter()
            .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect();
        assert_eq!(norm, ["cc -c -o src/a.o src/a.c"]);
    }

    #[test]
    fn recipes_prefixes_target_vars_oneshell() {
        let a = an(
            "V := $(shell git describe)\nbuild: CGO := 0\nbuild:\n\t@-echo $(CGO)\n\t+go build \
             -ldflags \"-X main.v=$(V)\" -o $(OUT) ./cmd/x\n",
        );
        let (lines, _, _) = a.recipe_lines(&a.targets["build"]);
        assert_eq!(
            lines,
            [
                "echo 0",
                "go build -ldflags \"-X main.v=__make_shell__\" -o ${__make_OUT} ./cmd/x"
            ]
        );
        let a = an(".ONESHELL:\nx:\n\tcd sub\n\tnode gen.js\n");
        let (lines, _, _) = a.recipe_lines(&a.targets["x"]);
        assert_eq!(lines, ["cd sub\nnode gen.js"]);
    }

    #[test]
    fn multiline_variable_recipes() {
        let a = an(
            "define BUILD\nnode a.js\nnode b.js\nendef\nall:\n\t$(BUILD)\n\techo \\\n\t  done\n",
        );
        let (lines, _, _) = a.recipe_lines(&a.targets["all"]);
        assert_eq!(lines, ["node a.js", "node b.js", "echo \\\n  done"]);
    }

    #[test]
    fn double_colon_and_merge() {
        let a = an("all:: a\n\techo 1\nall:: b\n\techo 2\nx: p1\nx: p2\n\ttouch x\n");
        assert_eq!(a.targets["all"].recipes.len(), 2);
        assert_eq!(a.targets["x"].prereqs, ["p1", "p2"]);
    }

    #[test]
    fn eval_is_global_uncertainty() {
        let a = an("$(eval $(call T,x))\nall:\n\techo\n");
        assert!(!a.db.issues.is_empty());
    }

    #[test]
    fn recipe_split_and_prefix() {
        assert_eq!(strip_recipe_prefixes("  @-+ echo"), "echo");
        assert_eq!(split_recipe("a\\\nb\nc"), ["a\\\nb", "c"]);
    }
}
