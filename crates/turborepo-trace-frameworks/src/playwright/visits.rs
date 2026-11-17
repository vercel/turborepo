//! Static extraction of the routes a Playwright spec visits (a Rust port of the
//! research prototype `extract-visits.mjs`, over lang-js tokens and the URL
//! evaluator in [`crate::nextjs::jseval`]).
//!
//! Visits come from:
//!
//! | Call | Visit |
//! |---|---|
//! | `page.goto(url)` | [`VisitKind::Goto`] |
//! | `po.goto(...)`, `po.gotoX(...)`, `po.open*/visit*/navigate*` on a page object | [`VisitKind::PageObject`]: the URLs its methods build (`path()`, `*Path`, `*Url`) and the `this.page.goto(...)` calls in the method |
//! | `expect(page).toHaveURL(x)`, `page.waitForURL(x)` | [`VisitKind::UrlAssert`] (strings and globs; regexes are listed as unresolved) |
//! | `page.route(x, ...)` | [`VisitKind::Route`] |
//! | `request.get(url)` / `page.request.post(url)` | [`VisitKind::Request`] |
//! | `.click()` on a locator whose selector names a link target (`a[href="/x"]`) | [`VisitKind::Click`] (a soft navigation) |
//! | `test.use({ routes: [...] })` / `test.use({ extraRoutes: [...] })` | [`Annotation`]: route patterns that replace / extend the extracted visits |
//!
//! First-party helper modules the spec imports are scanned too (their
//! navigations count for the spec), except files that define classes: page
//! objects contribute through calls on page-object variables only. Navigation
//! arguments that cannot be pinned down are kept in [`SpecVisits::unresolved`];
//! selection treats a spec with unresolved navigations as "may visit any
//! route".

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    rc::Rc,
};

use serde::Serialize;
use turborepo_trace_lang_js::lexer::TokKind;

use crate::nextjs::{
    jseval::{CallSite, Eval, Files, JsFile, Tpl, all_unknown, call_sites},
    source::Source,
    urlpat::{Piece, UrlPat},
};

/// How a visit was found.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VisitKind {
    /// `page.goto`.
    Goto,
    /// A page-object navigation.
    PageObject,
    /// `toHaveURL` / `waitForURL`.
    UrlAssert,
    /// `page.route`.
    Route,
    /// An API request from the test.
    Request,
    /// A click on a link with a known target.
    Click,
}

impl VisitKind {
    /// Whether an unresolved visit of this kind makes the spec incomplete.
    pub fn counts_for_completeness(self) -> bool {
        matches!(
            self,
            VisitKind::Goto | VisitKind::PageObject | VisitKind::Request
        )
    }
}

/// One visit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Visit {
    /// Kind.
    pub kind: VisitKind,
    /// The file the call is in.
    pub file: PathBuf,
    /// 1-based line.
    pub line: u32,
    /// The URL pattern.
    pub url: UrlPat,
    /// A client-side (soft) navigation.
    pub soft: bool,
    /// The unknown parts of the URL: the source text and where its value comes
    /// from ([`crate::nextjs::source::Source::name`]).
    #[serde(skip)]
    pub unknown: Vec<(String, &'static str)>,
    /// Every unknown part is a name of data: a fixture of the test callback
    /// (`async ({ page, team }) => page.goto(`/${team.slug}`)`), an environment
    /// variable, the result of a call or a response. Such a value names a
    /// team, a project or an id, not a route directory. An argument that no
    /// caller binds is not data: it can be any string the code passes, a
    /// fixed segment name included.
    #[serde(skip)]
    pub data_only: bool,
}

/// A navigation whose URL could not be determined.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct UnresolvedNav {
    /// Kind.
    pub kind: VisitKind,
    /// File.
    pub file: PathBuf,
    /// Line.
    pub line: u32,
    /// The argument text or reason.
    pub text: String,
}

/// `test.use({ routes })` / `test.use({ extraRoutes })`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Annotation {
    /// Route patterns (`/[team]/settings`).
    pub routes: Vec<String>,
    /// `true` for `routes` (replaces extraction), `false` for `extraRoutes`.
    pub replace: bool,
    /// Entries that are not string literals.
    pub unreadable: Vec<String>,
}

/// The visits of one spec.
#[derive(Clone, Debug, Default, Serialize)]
pub struct SpecVisits {
    /// The spec file.
    pub spec: PathBuf,
    /// Visits.
    pub visits: Vec<Visit>,
    /// Unresolved navigations.
    pub unresolved: Vec<UnresolvedNav>,
    /// Helper modules that contributed navigations.
    pub helper_files: Vec<PathBuf>,
    /// The annotation, if any.
    pub annotation: Option<Annotation>,
}

impl SpecVisits {
    /// Whether every navigation is known (an annotation that replaces
    /// extraction counts as complete).
    pub fn complete(&self) -> bool {
        if let Some(a) = &self.annotation
            && a.replace
            && a.unreadable.is_empty()
            && !a.routes.is_empty()
        {
            return true;
        }
        let navs = self
            .visits
            .iter()
            .filter(|v| v.kind.counts_for_completeness())
            .count();
        navs > 0
            && !self
                .unresolved
                .iter()
                .any(|u| u.kind.counts_for_completeness())
    }
}

/// Extraction options.
#[derive(Clone, Debug, Default)]
pub struct ExtractOptions {
    /// How many levels of imported helper modules to scan.
    pub helper_depth: u32,
}

/// Converts a Playwright URL glob (`**/team/*/settings`, `/x/**`) to pieces: a
/// leading `**/` is read as "any origin" (heuristic), `*` as an unknown segment
/// part, a trailing `**` as an open tail.
fn glob_pieces(g: &str) -> Vec<Piece> {
    let mut s = g;
    let mut out = Vec::new();
    if let Some(r) = s.strip_prefix("**") {
        out.push(Piece::Wild {
            multi: true,
            reason: "baseURL".into(),
            src: Default::default(),
        });
        s = r;
    }
    let mut lit = String::new();
    let b: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < b.len() {
        if b[i] == '*' {
            if !lit.is_empty() {
                out.push(Piece::Lit(std::mem::take(&mut lit)));
            }
            let multi = b.get(i + 1) == Some(&'*');
            out.push(Piece::Wild {
                multi,
                reason: if multi { "path".into() } else { "glob".into() },
                src: Default::default(),
            });
            i += if multi { 2 } else { 1 };
            continue;
        }
        lit.push(b[i]);
        i += 1;
    }
    if !lit.is_empty() {
        out.push(Piece::Lit(lit));
    }
    out
}

struct Ctx<'a, 'r> {
    files: &'a Files<'r>,
    out: SpecVisits,
    seen_methods: HashSet<(PathBuf, String, String)>,
    /// Fixture names of the test callbacks, per file.
    fixtures: std::collections::HashMap<PathBuf, Rc<HashSet<String>>>,
}

impl Ctx<'_, '_> {
    fn eval(&self) -> Eval<'_, '_> {
        Eval::new(self.files)
    }

    fn add(
        &mut self,
        kind: VisitKind,
        f: &JsFile,
        line: u32,
        tpls: &[Tpl],
        soft: bool,
        text: &str,
    ) {
        // An argument the evaluator has no value for at all (a `let` that a hook
        // assigns) is as unknown as one it gives up on.
        let no_value = tpls.is_empty() && matches!(kind, VisitKind::Goto | VisitKind::Request);
        if no_value || all_unknown(tpls) {
            self.out.unresolved.push(UnresolvedNav {
                kind,
                file: f.path.clone(),
                line,
                text: text.to_string(),
            });
            return;
        }
        for t in tpls {
            match UrlPat::from_pieces_lenient(t) {
                Ok(url) => {
                    let unknown: Vec<(String, &'static str)> = t
                        .iter()
                        .filter_map(|p| match p {
                            Piece::Lit(_) => None,
                            Piece::Wild { reason, src, .. } => Some((reason.clone(), src.name())),
                            Piece::Param(p) => Some((p.name.clone(), "param")),
                        })
                        .collect();
                    let fixtures = if unknown.is_empty() {
                        Rc::new(HashSet::new())
                    } else {
                        self.fixtures
                            .entry(f.path.clone())
                            .or_insert_with(|| {
                                Rc::new(fixture_names(f).into_iter().map(String::from).collect())
                            })
                            .clone()
                    };
                    let data_only = !unknown.is_empty()
                        && t.iter().all(|p| match p {
                            Piece::Lit(_) => true,
                            Piece::Wild { reason, src, .. } => {
                                matches!(src, Source::Env | Source::Call | Source::Response)
                                    || fixtures.contains(root_identifier(reason))
                            }
                            Piece::Param(_) => false,
                        });
                    let v = Visit {
                        kind,
                        file: f.path.clone(),
                        line,
                        url,
                        soft,
                        unknown,
                        data_only,
                    };
                    if !self.out.visits.iter().any(|x| {
                        x.kind == v.kind && x.url == v.url && x.soft == v.soft && x.file == v.file
                    }) {
                        self.out.visits.push(v);
                    }
                }
                Err(e) => self.out.unresolved.push(UnresolvedNav {
                    kind,
                    file: f.path.clone(),
                    line,
                    text: e.reason,
                }),
            }
        }
    }

    /// URLs a page-object method navigates to: `this.page.goto(x)` calls in its
    /// body and, when it calls other methods of the class, theirs (one
    /// level).
    fn class_navs(&mut self, f: &Rc<JsFile>, cls: &str, method: &str, depth: u32) -> Vec<Tpl> {
        if depth > 3 {
            return Vec::new();
        }
        let key = (f.path.clone(), cls.to_string(), method.to_string());
        if !self.seen_methods.insert(key) {
            return Vec::new();
        }
        let mut cur = self.files.find_class(f, cls);
        let mut out = Vec::new();
        let mut guard = 0;
        while let Some((cf, ci)) = cur {
            guard += 1;
            if guard > 8 {
                break;
            }
            let c = cf.classes[ci].clone();
            if let Some((_, (a, b))) = c.bodies.iter().find(|(n, _)| n == method) {
                let (a, b) = (*a, *b);
                for cs in call_sites(&cf)
                    .into_iter()
                    .filter(|cs| cs.at >= a && cs.at < b)
                {
                    if cs.method == "goto" && cs.receiver.as_deref() == Some("page") {
                        if let Some(&(x, y)) = cs.args.first() {
                            out.extend(self.eval().eval(&cf, x, y));
                        }
                    } else if cs.receiver.as_deref() == Some("this") && cs.method != method {
                        out.extend(self.class_navs(&cf, &c.name, &cs.method, depth + 1));
                    } else if cs.method == "click" {
                        self.click(&cf, &cs);
                    }
                }
                break;
            }
            cur = c.parent.and_then(|p| self.files.find_class(&cf, &p));
        }
        out
    }

    /// All URLs a page object builds (its `path`/`url` methods and fields, and
    /// every `goto` in its methods).
    fn class_paths(&mut self, f: &Rc<JsFile>, cls: &str) -> Vec<Tpl> {
        let mut out = Vec::new();
        if let Some(v) = self.eval().class_method(f, cls, "path", 0) {
            out.extend(v);
        }
        if out.is_empty() {
            let mut cur = self.files.find_class(f, cls);
            let mut guard = 0;
            while let Some((cf, ci)) = cur {
                guard += 1;
                if guard > 8 {
                    break;
                }
                let c = cf.classes[ci].clone();
                let names: Vec<String> = c
                    .methods
                    .iter()
                    .map(|(n, _)| n.clone())
                    .chain(c.fields.iter().map(|(n, _)| n.clone()))
                    .collect();
                for m in &names {
                    let lm = m.to_ascii_lowercase();
                    if (lm.contains("path") || lm.ends_with("url") || lm.ends_with("href"))
                        && let Some(v) = self.eval().class_method(&cf, &c.name, m, 0)
                    {
                        out.extend(v);
                    }
                }
                if out.is_empty() {
                    for (m, _) in &c.bodies {
                        if m.starts_with("goto") {
                            out.extend(self.class_navs(&cf, &c.name, m, 0));
                        }
                    }
                }
                cur = c.parent.and_then(|p| self.files.find_class(&cf, &p));
            }
        }
        out
    }

    /// A `.click()` whose receiver chain contains a selector with `href`.
    fn click(&mut self, f: &Rc<JsFile>, cs: &CallSite) {
        // Look back from the callee start over the statement for string tokens.
        let mut k = cs.at;
        let mut steps = 0;
        while k > 0 && steps < 40 {
            k -= 1;
            steps += 1;
            match f.kind(k) {
                TokKind::Semi | TokKind::LBrace | TokKind::RBrace => break,
                TokKind::Str | TokKind::Template => {
                    let s = f.str_value(k).unwrap_or_default();
                    if let Some(target) = href_in_selector(&s) {
                        let tpls = vec![target];
                        self.add(VisitKind::Click, f, f.line(cs.at), &tpls, true, &s);
                    }
                }
                _ => {}
            }
        }
    }

    fn scan_file(&mut self, f: &Rc<JsFile>, is_spec: bool) -> bool {
        let before = self.out.visits.len() + self.out.unresolved.len();
        for cs in call_sites(f) {
            let line = f.line(cs.at);
            let recv = cs.receiver.clone().unwrap_or_default();
            let po = f.var_class.get(&recv).cloned();
            let m = cs.method.as_str();
            let first = cs.args.first().copied();
            let arg_text = first.map(|(a, b)| f.range_text(a, b)).unwrap_or_default();
            if let Some(cls) = &po {
                let lm = m.to_ascii_lowercase();
                if m == "goto"
                    || lm.starts_with("goto")
                    || lm.starts_with("navigate")
                    || lm.starts_with("open")
                    || lm.starts_with("visit")
                {
                    let mut tpls: Vec<Tpl> = Vec::new();
                    if m == "goto"
                        && let Some((a, b)) = first
                    {
                        let v = self.eval().eval(f, a, b);
                        if !all_unknown(&v) {
                            tpls.extend(v);
                        }
                    }
                    let navs = self.class_navs(f, cls, m, 0);
                    tpls.extend(
                        navs.into_iter()
                            .filter(|t| !(t.len() == 1 && matches!(t[0], Piece::Wild { .. }))),
                    );
                    if tpls.is_empty() {
                        tpls = self.class_paths(f, cls);
                    }
                    let tpls: Vec<Tpl> = tpls.into_iter().take(8).collect();
                    self.add(
                        VisitKind::PageObject,
                        f,
                        line,
                        &tpls,
                        false,
                        &format!("{recv}.{m}()"),
                    );
                    continue;
                }
                // Other page-object methods: their gotos and link clicks.
                let navs = self.class_navs(f, cls, m, 0);
                if !navs.is_empty() {
                    self.add(
                        VisitKind::PageObject,
                        f,
                        line,
                        &navs,
                        false,
                        &format!("{recv}.{m}()"),
                    );
                }
                continue;
            }
            match m {
                "goto" if cs.receiver.is_some() => {
                    if let Some((a, b)) = first {
                        let v = self.eval().eval(f, a, b);
                        self.add(VisitKind::Goto, f, line, &v, false, &arg_text);
                    }
                }
                "toHaveURL" | "waitForURL" => {
                    let Some((a, b)) = first else { continue };
                    if f.kind(a) == TokKind::Regex && b == a + 1 {
                        self.out.unresolved.push(UnresolvedNav {
                            kind: VisitKind::UrlAssert,
                            file: f.path.clone(),
                            line,
                            text: arg_text,
                        });
                        continue;
                    }
                    let v = self.eval().eval(f, a, b);
                    let v = globbed(v);
                    self.add(VisitKind::UrlAssert, f, line, &v, true, &arg_text);
                }
                "route" if matches!(recv.as_str(), "page" | "context") => {
                    let Some((a, b)) = first else { continue };
                    if f.kind(a) == TokKind::Regex {
                        self.out.unresolved.push(UnresolvedNav {
                            kind: VisitKind::Route,
                            file: f.path.clone(),
                            line,
                            text: arg_text,
                        });
                        continue;
                    }
                    let v = globbed(self.eval().eval(f, a, b));
                    self.add(VisitKind::Route, f, line, &v, false, &arg_text);
                }
                "get" | "post" | "put" | "patch" | "delete" | "head" | "fetch"
                    if recv == "request" || recv.ends_with("Request") || recv == "apiContext" =>
                {
                    if let Some((a, b)) = first {
                        let v = self.eval().eval(f, a, b);
                        self.add(VisitKind::Request, f, line, &v, false, &arg_text);
                    }
                }
                "click" => self.click(f, &cs),
                "use" if is_spec && recv == "test" => {
                    if let Some((a, b)) = first {
                        self.annotation(f, a, b);
                    }
                }
                _ => {}
            }
        }
        self.out.visits.len() + self.out.unresolved.len() > before
    }

    fn annotation(&mut self, f: &Rc<JsFile>, a: usize, b: usize) {
        if f.kind(a) != TokKind::LBrace || f.closer(a) != Some(b - 1) {
            return;
        }
        for (x, y) in f.split_commas(a + 1, b - 1) {
            let key = f.text(x);
            if !matches!(key, "routes" | "extraRoutes") || f.kind(x + 1) != TokKind::Colon {
                continue;
            }
            let ann = self.out.annotation.get_or_insert_with(Annotation::default);
            ann.replace |= key == "routes";
            let (p, q) = (x + 2, y);
            if f.kind(p) != TokKind::LBracket {
                ann.unreadable.push(f.range_text(p, q));
                continue;
            }
            let c = f.closer(p).unwrap_or(q);
            for (s, e) in f.split_commas(p + 1, c) {
                match (f.kind(s), e == s + 1) {
                    (TokKind::Str | TokKind::Template, true) => {
                        if let Some(v) = f.str_value(s) {
                            ann.routes.push(v);
                        }
                    }
                    _ => ann.unreadable.push(f.range_text(s, e)),
                }
            }
        }
    }
}

/// The names a test callback takes from its fixtures: the destructured first
/// parameter of a function passed to the test runner (`test('x', async ({ page,
/// team }) => …)`, hooks, steps, `test.extend({ f: async ({ page }, use) => …
/// })`).
fn fixture_names(f: &JsFile) -> HashSet<&str> {
    let mut out = HashSet::new();
    for fd in &f.funcs {
        let Some(callee) = f.enclosing_call(fd.start) else {
            continue;
        };
        if !matches!(
            f.text(callee),
            "test"
                | "it"
                | "beforeEach"
                | "afterEach"
                | "beforeAll"
                | "afterAll"
                | "step"
                | "describe"
                | "only"
                | "skip"
                | "fixme"
                | "fail"
                | "slow"
                | "extend"
        ) {
            continue;
        }
        out.extend(
            fd.params
                .iter()
                .filter(|p| p.index == 0 && p.field.is_some())
                .map(|p| p.name.as_str()),
        );
    }
    out
}

/// The identifier an expression starts with (`team` of `team.slug`).
fn root_identifier(expr: &str) -> &str {
    let end = expr
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$'))
        .unwrap_or(expr.len());
    &expr[..end]
}

/// How many levels of helper modules are walked at most ([`extract_visits`]).
const MAX_HELPER_DEPTH: u32 = 6;

/// Expands Playwright URL globs in string alternatives.
fn globbed(v: Vec<Tpl>) -> Vec<Tpl> {
    v.into_iter()
        .map(|t| {
            let mut out = Vec::new();
            for p in t {
                match p {
                    Piece::Lit(s) if s.contains('*') => out.extend(glob_pieces(&s)),
                    p => out.push(p),
                }
            }
            out
        })
        .collect()
}

/// The link target named in a selector: `a[href="/x"]` (exact), `a[href^="/x"]`
/// (prefix).
fn href_in_selector(s: &str) -> Option<Tpl> {
    let i = s.find("href")?;
    let rest = &s[i + 4..];
    let (op, rest) = if let Some(r) = rest.strip_prefix("^=") {
        ('^', r)
    } else {
        let r = rest.strip_prefix('=')?;
        ('=', r)
    };
    let q = rest.chars().next()?;
    let (val, _) = if q == '"' || q == '\'' {
        let r = &rest[1..];
        let e = r.find(q)?;
        (&r[..e], ())
    } else {
        let e = rest.find(']')?;
        (&rest[..e], ())
    };
    if !val.starts_with('/') {
        return None;
    }
    let mut t = vec![Piece::Lit(val.to_string())];
    if op == '^' {
        t.push(Piece::Wild {
            multi: true,
            reason: "path".into(),
            src: Default::default(),
        });
    }
    Some(t)
}

/// What scanning a helper module found, kept across the specs that import it.
#[derive(Default)]
pub struct HelperCache(std::cell::RefCell<std::collections::HashMap<PathBuf, Rc<SpecVisits>>>);

/// Extracts the visits of `spec`.
pub fn extract_visits(spec: &Path, files: &Files<'_>, opts: &ExtractOptions) -> SpecVisits {
    extract_visits_cached(spec, files, opts, &HelperCache::default())
}

/// [`extract_visits`] for many specs: a helper module that several specs import
/// is scanned once.
pub fn extract_visits_cached(
    spec: &Path,
    files: &Files<'_>,
    opts: &ExtractOptions,
    cache: &HelperCache,
) -> SpecVisits {
    let mut ctx = Ctx {
        files,
        out: SpecVisits {
            spec: spec.to_path_buf(),
            ..SpecVisits::default()
        },
        seen_methods: HashSet::new(),
        fixtures: std::collections::HashMap::new(),
    };
    let Some(f) = files.get(spec) else {
        ctx.out.unresolved.push(UnresolvedNav {
            kind: VisitKind::Goto,
            file: spec.to_path_buf(),
            line: 0,
            text: "spec unreadable".into(),
        });
        return ctx.out;
    };
    ctx.scan_file(&f, true);
    // Helper modules (not page objects), breadth first.
    // Two levels of whatever the spec imports; further (a helper package behind its
    // index files) only through barrels and modules that import Playwright,
    // which is where a helper that navigates lives.
    let depth = if opts.helper_depth == 0 {
        MAX_HELPER_DEPTH
    } else {
        opts.helper_depth
    };
    let mut seen: HashSet<PathBuf> = HashSet::from([spec.to_path_buf()]);
    let mut frontier = vec![f];
    for level in 0..depth {
        let mut next = Vec::new();
        for g in &frontier {
            let modules = g
                .imports
                .iter()
                .map(|i| i.module.as_str())
                .chain(g.reexports.iter().map(|r| r.module.as_str()));
            for module in modules {
                let Some(p) = files.resolve(&g.path, module) else {
                    continue;
                };
                if !seen.insert(p.clone()) {
                    continue;
                }
                let Some(h) = files.get(&p) else { continue };
                if level >= 2
                    && h.reexports.is_empty()
                    && !h.imports.iter().any(|i| i.module.contains("playwright"))
                {
                    continue;
                }
                if !h.classes.is_empty() {
                    continue;
                }
                // What a helper contributes does not depend on the spec that imports it.
                let found = cache.0.borrow().get(&p).cloned();
                let found = match found {
                    Some(f) => f,
                    None => {
                        let mut sub = Ctx {
                            files,
                            out: SpecVisits::default(),
                            seen_methods: HashSet::new(),
                            fixtures: std::collections::HashMap::new(),
                        };
                        sub.scan_file(&h, false);
                        let f = Rc::new(sub.out);
                        cache.0.borrow_mut().insert(p.clone(), f.clone());
                        f
                    }
                };
                if !found.visits.is_empty() || !found.unresolved.is_empty() {
                    for v in &found.visits {
                        if !ctx.out.visits.iter().any(|x| {
                            x.kind == v.kind
                                && x.url == v.url
                                && x.soft == v.soft
                                && x.file == v.file
                        }) {
                            ctx.out.visits.push(v.clone());
                        }
                    }
                    ctx.out.unresolved.extend(found.unresolved.iter().cloned());
                    ctx.out.helper_files.push(p);
                }
                next.push(h);
            }
        }
        frontier = next;
    }
    ctx.out
}
