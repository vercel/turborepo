//! URL → route matching with Next's layering and precedence:
//!
//! basePath → i18n locale → microfrontends → middleware (noted) → redirects →
//! `beforeFiles` rewrites → `public/` files and static routes → `afterFiles`
//! rewrites → dynamic routes → `fallback` rewrites.
//!
//! Within a layer, routes rank segment by segment: static < dynamic < catch-all
//! < optional catch-all (Next's sorter). Intercepting routes only take part in
//! soft navigations; slot pages only when no `children` page matches.
//!
//! Inputs are [`UrlPat`]s, so templates work: a segment with unknown parts
//! matches by the **heuristic** that an unknown value never equals a fixed
//! segment name ([`UrlMatch::routes`]). The **sound** set
//! ([`UrlMatch::candidates`]) also keeps every route some instantiation could
//! hit, minus routes that can never win. When the two differ, or a rule's
//! outcome depends on cookies, headers or the environment, the result
//! carries an explicit [`Ambiguity`] instead of a silent pick.

use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
};

use serde::Serialize;

use super::{
    config::{Cond, NextConfig, Rule},
    mfe::{Microfrontends, Middleware},
    p2r::compile_destination,
    routes::{RouteKind, RouteTable, Router, Seg},
    urlpat::{Origin, Part, Unresolved, UrlPat, always_matches, overlaps},
};
use crate::fsx::PathFsx;

const SENTINEL: &str = "__dtp__";
const MAX_DEPTH: u32 = 6;

/// How many segments a middleware is assumed to put in front of a path at most.
pub const MAX_PREFIX: usize = 3;

/// The routes a path can reach behind segments that a middleware put in front
/// of it ([`Matcher::behind_prefix`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prefixed {
    /// The number of segments in front.
    pub k: usize,
    /// Route ids by Next's precedence, when the segments in front are values of
    /// leading dynamic segments (a locale, a flags code, a tenant).
    pub likely: Vec<String>,
    /// Route ids the prefixed path can hit whatever the segments in front are
    /// (a superset of `likely`).
    pub sound: Vec<String>,
}

/// The request context a test runs in, for `has`/`missing` conditions.
#[derive(Clone, Debug, Serialize)]
pub struct RequestContext {
    /// Host (`localhost`).
    pub host: String,
    /// Known cookies.
    pub cookies: BTreeMap<String, String>,
    /// Known headers (lower-case names).
    pub headers: BTreeMap<String, String>,
}

impl Default for RequestContext {
    fn default() -> Self {
        RequestContext {
            host: "localhost".into(),
            cookies: BTreeMap::new(),
            headers: BTreeMap::new(),
        }
    }
}

/// Where a URL ends up.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "detail")]
pub enum Outcome {
    /// One or more local routes.
    Routes,
    /// An external origin (absolute URL, or a rule to one).
    External(String),
    /// Another microfrontends application.
    Microfrontend(String),
    /// A file in `public/`.
    Public(String),
    /// No route (404).
    NotFound,
}

/// Why a match is not a single certain route.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Ambiguity {
    /// An unknown segment could equal a fixed segment name: the sound set has
    /// more routes.
    ParamMayEqualStatic {
        /// The extra route ids.
        extra: Vec<String>,
    },
    /// The pattern has an open tail (a URL prefix): it overlaps several routes.
    OpenPattern,
    /// A rule's conditions (cookie, header, query, environment) are unknown:
    /// both outcomes were kept.
    ConditionalRule {
        /// `redirect`, `beforeFiles`, `afterFiles`, `fallback`.
        phase: String,
        /// The rule source.
        source: String,
    },
    /// A rule could not be compiled; it may or may not apply.
    UnsupportedRule {
        /// The rule source.
        source: String,
    },
    /// Not every redirect/rewrite of the config could be read.
    RulesIncomplete,
    /// A microfrontends group gated by a flag matched.
    MicrofrontendFlag {
        /// The other app.
        app: String,
        /// The flag.
        flag: String,
    },
    /// The same URL exists in `app/` and `pages/` (a build error).
    RouterConflict,
    /// The middleware runs for this path and may rewrite or redirect it in
    /// code.
    Middleware,
}

/// The result of matching one URL pattern.
#[derive(Clone, Debug, Serialize)]
pub struct UrlMatch {
    /// The input.
    pub input: String,
    /// The path after rules.
    pub target: String,
    /// Outcome.
    pub outcome: Outcome,
    /// Route ids, heuristic (sorted).
    pub routes: Vec<String>,
    /// Route ids, sound (a superset of `routes`, sorted).
    pub candidates: Vec<String>,
    /// Ambiguities.
    pub ambiguities: Vec<Ambiguity>,
    /// Rules applied (`redirect:/old/:p`, `rewrite:afterFiles:/a`).
    pub via: Vec<String>,
}

/// A matcher over one app.
pub struct Matcher<'a> {
    table: &'a RouteTable,
    config: &'a NextConfig,
    mfe: &'a Microfrontends,
    middleware: &'a Middleware,
    /// Rule indexes: redirects, `beforeFiles`, `afterFiles`, `fallback`.
    rules: [RuleIndex; 4],
    /// The middleware's `config.matcher` patterns, compiled once (`None`:
    /// unsupported, the middleware may run).
    mw_patterns: Option<Vec<Option<super::p2r::PathPattern>>>,
    public_dir: Option<PathBuf>,
    ctx: RequestContext,
}

fn cond_known(c: &Cond, ctx: &RequestContext) -> Option<bool> {
    let test = |v: &str| match &c.value {
        None => true,
        Some(re) => regex::Regex::new(&format!("^(?:{re})$")).is_ok_and(|r| r.is_match(v)),
    };
    match c.kind.as_str() {
        "host" => Some(test(&ctx.host)),
        "cookie" => c
            .key
            .as_deref()
            .and_then(|k| ctx.cookies.get(k))
            .map(|v| test(v)),
        "header" => c
            .key
            .as_deref()
            .and_then(|k| ctx.headers.get(&k.to_ascii_lowercase()))
            .map(|v| test(v)),
        _ => None,
    }
}

/// `Some(true)` applies, `Some(false)` does not, `None` unknown.
fn conditions(rule: &Rule, ctx: &RequestContext) -> Option<bool> {
    let mut unknown = rule.conditional;
    for c in &rule.has {
        match cond_known(c, ctx) {
            Some(false) => return Some(false),
            None => unknown = true,
            _ => {}
        }
    }
    for c in &rule.missing {
        match cond_known(c, ctx) {
            Some(true) => return Some(false),
            None => unknown = true,
            _ => {}
        }
    }
    if unknown { None } else { Some(true) }
}

enum RuleHit {
    No,
    Unsupported,
    Yes(Option<String>),
}

/// The first path segment of a rule source when it is plain text
/// (`/docs/:path*` → `docs`, lower-cased): the rule can only match paths whose
/// first segment is this one, so most rules of a long redirect list are ruled
/// out without running their pattern.
fn literal_first(source: &str) -> Option<String> {
    let rest = source.strip_prefix('/')?;
    let seg = rest.split('/').next()?;
    if seg.is_empty()
        || !seg.is_ascii()
        || seg.chars().any(|c| {
            matches!(
                c,
                ':' | '(' | ')' | '{' | '}' | '*' | '+' | '?' | '\\' | '[' | ']' | '|' | '.'
            )
        })
    {
        return None;
    }
    Some(seg.to_ascii_lowercase())
}

/// The rules of one list by their plain first segment, so that a path is tried
/// only against the rules that can match it (a redirect list can have
/// thousands).
#[derive(Default)]
struct RuleIndex {
    /// Lower-cased first segment → rule indexes (ascending).
    by_first: HashMap<String, Vec<u32>>,
    /// Rules without a plain first segment (ascending): tried for every path.
    any: Vec<u32>,
}

impl RuleIndex {
    fn new(rules: &[Rule]) -> RuleIndex {
        let mut ix = RuleIndex::default();
        for (i, r) in rules.iter().enumerate() {
            match literal_first(&r.source).filter(|_| r.pattern.is_some()) {
                Some(l) => ix.by_first.entry(l).or_default().push(i as u32),
                None => ix.any.push(i as u32),
            }
        }
        ix
    }

    /// The rules that can match `rendered`, in list order.
    fn candidates<'r>(&self, rules: &'r [Rule], rendered: &str) -> Vec<&'r Rule> {
        let rest = rendered.strip_prefix('/').unwrap_or(rendered);
        let seg = rest.split('/').next().unwrap_or("").to_ascii_lowercase();
        let named: &[u32] = self.by_first.get(&seg).map_or(&[], Vec::as_slice);
        let (mut a, mut b) = (0, 0);
        let mut out = Vec::with_capacity(named.len() + self.any.len());
        while a < named.len() || b < self.any.len() {
            let i = match (named.get(a), self.any.get(b)) {
                (Some(&x), Some(&y)) if x < y => {
                    a += 1;
                    x
                }
                (Some(&x), None) => {
                    a += 1;
                    x
                }
                (_, Some(&y)) => {
                    b += 1;
                    y
                }
                (None, None) => break,
            };
            if let Some(r) = rules.get(i as usize) {
                out.push(r);
            }
        }
        out
    }
}

fn apply(rule: &Rule, rendered: &str) -> RuleHit {
    let Some(p) = &rule.pattern else {
        let lit: String = rule
            .source
            .chars()
            .take_while(|c| !matches!(c, ':' | '(' | '{' | '*'))
            .collect();
        return if rendered.starts_with(&lit) {
            RuleHit::Unsupported
        } else {
            RuleHit::No
        };
    };
    if !p.is_match(rendered) {
        return RuleHit::No;
    }
    match p.matches(rendered) {
        None => RuleHit::No,
        Some(params) => {
            let dest = rule.destination.split(['?', '#']).next().unwrap_or("");
            RuleHit::Yes(compile_destination(dest, &params))
        }
    }
}

fn cmp_rank(a: &[u8], b: &[u8]) -> std::cmp::Ordering {
    a.cmp(b)
}

impl<'a> Matcher<'a> {
    /// A matcher.
    pub fn new(
        table: &'a RouteTable,
        config: &'a NextConfig,
        mfe: &'a Microfrontends,
        middleware: &'a Middleware,
        public_dir: Option<PathBuf>,
    ) -> Self {
        let rw = &config.rewrites;
        let rules = [
            RuleIndex::new(&config.redirects),
            RuleIndex::new(&rw.before_files),
            RuleIndex::new(&rw.after_files),
            RuleIndex::new(&rw.fallback),
        ];
        let mw_patterns = middleware.matcher.as_ref().map(|ms| {
            ms.iter()
                .map(|m| super::p2r::PathPattern::new(m).ok())
                .collect()
        });
        Matcher {
            table,
            config,
            mfe,
            middleware,
            rules,
            mw_patterns,
            public_dir,
            ctx: RequestContext::default(),
        }
    }

    /// Whether a redirect or rewrite rule may apply to `pat` (as the first step
    /// of [`Matcher::match_pat`]): if none does, the routes of `pat` are
    /// those its segments overlap.
    pub fn may_rewrite(&self, pat: &UrlPat) -> bool {
        let Some(p) = self.strip_base(pat) else {
            return false;
        };
        let rendered = p.render(SENTINEL);
        let rw = &self.config.rewrites;
        let lists: [&[Rule]; 4] = [
            &self.config.redirects,
            &rw.before_files,
            &rw.after_files,
            &rw.fallback,
        ];
        lists.iter().zip(&self.rules).any(|(rules, ix)| {
            ix.candidates(rules, &rendered)
                .into_iter()
                .any(|r| !matches!(apply(r, &rendered), RuleHit::No))
        })
    }

    /// [`Middleware::runs_for`] with the matcher patterns compiled once.
    fn middleware_runs_for(&self, path: &str) -> bool {
        if self.middleware.files.is_empty() {
            return false;
        }
        match &self.mw_patterns {
            None => true,
            Some(ps) => ps
                .iter()
                .any(|p| p.as_ref().is_none_or(|p| p.is_match(path))),
        }
    }

    /// What `url` can reach when the app's middleware rewrites it behind a
    /// prefix: for 1 to [`MAX_PREFIX`] segments in front of the path, the
    /// routes of the prefixed path. For a URL the middleware runs for
    /// ([`Ambiguity::Middleware`] of its own match): a request
    /// for `/x` under a middleware that rewrites to `/<locale>/x` is served by
    /// `/[locale]/x`, which the path alone does not match. The same for a URL a
    /// spec names and for the target of a link or a redirect in the app's
    /// code.
    pub fn behind_prefix(&self, url: &UrlPat, soft: bool) -> Vec<Prefixed> {
        (1..=MAX_PREFIX)
            .map(|k| {
                let mut prefixed = url.clone();
                let mut segs = vec![vec![Part::Any]; k];
                segs.extend(prefixed.segs);
                prefixed.segs = segs;
                let pm = self.match_pat(&prefixed, soft);
                let leading_dynamic = |id: &String| {
                    self.table
                        .routes
                        .iter()
                        .find(|r| r.id == *id)
                        .is_some_and(|r| {
                            r.segs.len() >= k
                                && r.segs[..k].iter().all(|s| matches!(s, Seg::Dynamic(_)))
                        })
                };
                Prefixed {
                    k,
                    likely: pm
                        .routes
                        .iter()
                        .filter(|id| leading_dynamic(id))
                        .cloned()
                        .collect(),
                    sound: pm.candidates,
                }
            })
            .collect()
    }

    /// Serves `public/` files from `dir`.
    pub fn with_public(mut self, dir: PathBuf) -> Self {
        self.public_dir = Some(dir);
        self
    }

    /// Sets the request context.
    pub fn with_context(mut self, ctx: RequestContext) -> Self {
        self.ctx = ctx;
        self
    }

    /// Matches a URL string (a path, or an absolute URL).
    pub fn match_str(&self, url: &str) -> Result<UrlMatch, Unresolved> {
        Ok(self.match_pat(&UrlPat::literal(url)?, false))
    }

    /// Matches a pattern. `soft`: a client-side navigation (intercepting routes
    /// apply).
    pub fn match_pat(&self, pat: &UrlPat, soft: bool) -> UrlMatch {
        let mut m = UrlMatch {
            input: pat.to_string(),
            target: pat.path_string(),
            outcome: Outcome::NotFound,
            routes: Vec::new(),
            candidates: Vec::new(),
            ambiguities: Vec::new(),
            via: Vec::new(),
        };
        if let Origin::Absolute(h) = &pat.origin {
            let host = h.split("://").nth(1).unwrap_or(h).trim_start_matches("//");
            let host = host.split(':').next().unwrap_or(host);
            if host != self.ctx.host {
                m.outcome = Outcome::External(h.clone());
                return m;
            }
        }
        if !self.config.rules_complete {
            m.ambiguities.push(Ambiguity::RulesIncomplete);
        }
        self.resolve(pat.clone(), soft, 0, &mut m);
        m.routes.sort();
        m.routes.dedup();
        m.candidates.extend(m.routes.clone());
        m.candidates.sort();
        m.candidates.dedup();
        if m.outcome == Outcome::NotFound && !m.routes.is_empty() {
            m.outcome = Outcome::Routes;
        }
        m.ambiguities.dedup();
        m
    }

    fn strip_base(&self, pat: &UrlPat) -> Option<UrlPat> {
        let mut p = pat.clone();
        if let Some(bp) = &self.config.base_path {
            let segs: Vec<&str> = bp.split('/').filter(|s| !s.is_empty()).collect();
            p = p.strip_prefix_segs(&segs)?;
        }
        if let Some(i18n) = &self.config.i18n
            && let Some(first) = p.lit(0)
            && i18n.locales.contains(&first)
        {
            p = p.strip_prefix_segs(&[first.as_str()])?;
        }
        Some(p)
    }

    fn resolve(&self, pat: UrlPat, soft: bool, depth: u32, m: &mut UrlMatch) {
        if depth > MAX_DEPTH {
            m.via.push("rule-loop".into());
            return;
        }
        let Some(p) = (if depth == 0 {
            self.strip_base(&pat)
        } else {
            Some(pat.clone())
        }) else {
            m.via.push("outside-basePath".into());
            return;
        };
        let rendered = p.render(SENTINEL);
        // Microfrontends.
        for o in &self.mfe.others {
            if o.pattern.as_ref().is_some_and(|x| x.is_match(&rendered)) {
                match &o.flag {
                    Some(f) => m.ambiguities.push(Ambiguity::MicrofrontendFlag {
                        app: o.app.clone(),
                        flag: f.clone(),
                    }),
                    None => {
                        m.outcome = Outcome::Microfrontend(o.app.clone());
                        m.via.push(format!("microfrontend:{}", o.source));
                        return;
                    }
                }
                break;
            }
        }
        if !self.mfe.own.is_empty()
            && !self
                .mfe
                .own
                .iter()
                .any(|o| o.pattern.as_ref().is_some_and(|x| x.is_match(&rendered)))
        {
            m.outcome = Outcome::Microfrontend("<default app>".into());
            return;
        }
        if depth == 0 && self.middleware_runs_for(&rendered) {
            m.ambiguities.push(Ambiguity::Middleware);
        }
        // Redirects.
        for r in self.rules[0].candidates(&self.config.redirects, &rendered) {
            match apply(r, &rendered) {
                RuleHit::No => {}
                RuleHit::Unsupported => m.ambiguities.push(Ambiguity::UnsupportedRule {
                    source: r.source.clone(),
                }),
                RuleHit::Yes(dest) => {
                    let cond = conditions(r, &self.ctx);
                    if cond == Some(false) {
                        continue;
                    }
                    let Some(dest) = dest else {
                        m.ambiguities.push(Ambiguity::UnsupportedRule {
                            source: r.source.clone(),
                        });
                        continue;
                    };
                    if cond.is_none() {
                        m.ambiguities.push(Ambiguity::ConditionalRule {
                            phase: "redirect".into(),
                            source: r.source.clone(),
                        });
                        // Keep the redirect target as a candidate, then go on.
                        let mut side = self.side(m);
                        self.follow(&dest, soft, depth, &mut side);
                        m.candidates.extend(side.routes);
                        m.candidates.extend(side.candidates);
                        continue;
                    }
                    m.via.push(format!("redirect:{}", r.source));
                    self.follow(&dest, soft, depth, m);
                    return;
                }
            }
        }
        let mut target = p;
        // beforeFiles.
        match self.rewrite(
            &self.config.rewrites.before_files,
            &self.rules[1],
            "beforeFiles",
            &target,
            soft,
            m,
        ) {
            Rw::Done => return,
            Rw::To(t) => target = t,
            Rw::None => {}
        }
        m.target = target.path_string();
        // Filesystem: public files and static routes.
        if target.is_concrete()
            && let Some(pd) = &self.public_dir
        {
            let rel = target.path_string();
            let f = pd.join(rel.trim_start_matches('/'));
            if rel != "/" && f.fx_is_file() {
                m.outcome = Outcome::Public(rel);
                return;
            }
        }
        if self.best(&target, true, soft, m) {
            return;
        }
        match self.rewrite(
            &self.config.rewrites.after_files,
            &self.rules[2],
            "afterFiles",
            &target,
            soft,
            m,
        ) {
            Rw::Done => return,
            Rw::To(t) => {
                m.target = t.path_string();
                if self.best(&t, false, soft, m) {
                    return;
                }
                target = t;
            }
            Rw::None => {}
        }
        if self.best(&target, false, soft, m) {
            return;
        }
        match self.rewrite(
            &self.config.rewrites.fallback,
            &self.rules[3],
            "fallback",
            &target,
            soft,
            m,
        ) {
            Rw::Done => {}
            Rw::To(t) => {
                m.target = t.path_string();
                self.best(&t, false, soft, m);
            }
            Rw::None => {}
        }
    }

    fn side(&self, m: &UrlMatch) -> UrlMatch {
        UrlMatch {
            input: m.input.clone(),
            target: String::new(),
            outcome: Outcome::NotFound,
            routes: Vec::new(),
            candidates: Vec::new(),
            ambiguities: Vec::new(),
            via: Vec::new(),
        }
    }

    fn follow(&self, dest: &str, soft: bool, depth: u32, m: &mut UrlMatch) {
        match UrlPat::parse_rendered(dest, SENTINEL) {
            Ok(p) => {
                if let Origin::Absolute(h) = &p.origin {
                    m.outcome = Outcome::External(h.clone());
                    return;
                }
                self.resolve(p, soft, depth + 1, m);
            }
            Err(_) => m.ambiguities.push(Ambiguity::UnsupportedRule {
                source: dest.to_string(),
            }),
        }
    }

    fn rewrite(
        &self,
        rules: &[Rule],
        index: &RuleIndex,
        phase: &str,
        target: &UrlPat,
        soft: bool,
        m: &mut UrlMatch,
    ) -> Rw {
        let rendered = target.render(SENTINEL);
        for r in index.candidates(rules, &rendered) {
            match apply(r, &rendered) {
                RuleHit::No => {}
                RuleHit::Unsupported => m.ambiguities.push(Ambiguity::UnsupportedRule {
                    source: r.source.clone(),
                }),
                RuleHit::Yes(dest) => {
                    let cond = conditions(r, &self.ctx);
                    if cond == Some(false) {
                        continue;
                    }
                    let Some(dest) = dest else {
                        m.ambiguities.push(Ambiguity::UnsupportedRule {
                            source: r.source.clone(),
                        });
                        continue;
                    };
                    let Ok(np) = UrlPat::parse_rendered(&dest, SENTINEL) else {
                        m.ambiguities.push(Ambiguity::UnsupportedRule {
                            source: r.source.clone(),
                        });
                        continue;
                    };
                    if cond.is_none() {
                        m.ambiguities.push(Ambiguity::ConditionalRule {
                            phase: phase.into(),
                            source: r.source.clone(),
                        });
                        if !matches!(np.origin, Origin::Absolute(_)) {
                            let mut side = self.side(m);
                            self.best(&np, false, soft, &mut side);
                            m.candidates.extend(side.routes);
                            m.candidates.extend(side.candidates);
                        }
                        continue;
                    }
                    m.via.push(format!("rewrite:{phase}:{}", r.source));
                    if let Origin::Absolute(h) = &np.origin {
                        m.outcome = Outcome::External(h.clone());
                        return Rw::Done;
                    }
                    return Rw::To(np);
                }
            }
        }
        Rw::None
    }

    /// Finds the best routes for `pat`; returns whether something matched.
    fn best(&self, pat: &UrlPat, static_only: bool, soft: bool, m: &mut UrlMatch) -> bool {
        let routes = &self.table.routes;
        // A route whose first segment is a fixed name other than the pattern's first
        // (fully known) segment cannot match: most routes are ruled out by one
        // comparison.
        let first = pat.lit(0);
        let eligible = |r: &&super::routes::Route| {
            (soft || r.intercept.is_none())
                && (!static_only || r.is_static())
                && !matches!(
                    (r.segs.first(), first.as_deref()),
                    (Some(super::routes::Seg::Static(v)), Some(l)) if v != l
                )
        };
        // The pattern's segments as known text or unknown, once for every route.
        let parts: Vec<Option<String>> = (0..pat.segs.len()).map(|i| pat.lit(i)).collect();
        let refs: Vec<Option<&str>> = parts.iter().map(|p| p.as_deref()).collect();
        // Heuristic.
        let mut best: Vec<(usize, Vec<u8>)> = Vec::new();
        for (i, r) in routes.iter().enumerate() {
            if !eligible(&r) || r.intercept.is_some() {
                continue;
            }
            if pat.open_tail {
                break;
            }
            let Some(rk) = super::routes::rank_against(&r.segs, &refs) else {
                continue;
            };
            match best.first() {
                None => best.push((i, rk)),
                Some((_, b)) => match cmp_rank(&rk, b) {
                    std::cmp::Ordering::Less => best = vec![(i, rk)],
                    std::cmp::Ordering::Equal => best.push((i, rk)),
                    _ => {}
                },
            }
        }
        let mut chosen: Vec<usize> = best.iter().map(|(i, _)| *i).collect();
        let primary: Vec<usize> = chosen
            .iter()
            .copied()
            .filter(|&i| routes[i].slot.is_none())
            .collect();
        if !primary.is_empty() {
            chosen = primary;
        }
        // Sound candidates.
        let cands: Vec<usize> = routes
            .iter()
            .enumerate()
            .filter(|(_, r)| eligible(r) && r.intercept.is_none() && r.slot.is_none())
            .filter(|(_, r)| overlaps(&r.segs, pat))
            .map(|(i, _)| i)
            .collect();
        let rank = |i: usize| -> Vec<u8> { routes[i].segs.iter().map(|s| s.rank()).collect() };
        let always: Vec<usize> = cands
            .iter()
            .copied()
            .filter(|&d| always_matches(&routes[d].segs, pat))
            .collect();
        let always_ranks: Vec<(usize, Vec<u8>)> = always.iter().map(|&d| (d, rank(d))).collect();
        let sound: Vec<usize> = cands
            .iter()
            .copied()
            .filter(|&c| {
                let rc = rank(c);
                !always_ranks
                    .iter()
                    .any(|(d, rd)| *d != c && cmp_rank(rd, &rc) == std::cmp::Ordering::Less)
            })
            .collect();
        if pat.open_tail && chosen.is_empty() {
            chosen = sound.clone();
            if chosen.len() > 1 {
                m.ambiguities.push(Ambiguity::OpenPattern);
            }
        }
        if chosen.is_empty() && sound.is_empty() {
            return false;
        }
        if soft && !chosen.is_empty() {
            for (i, r) in routes.iter().enumerate() {
                if r.intercept.is_some() && r.kind == RouteKind::Page && overlaps(&r.segs, pat) {
                    chosen.push(i);
                }
            }
        }
        let routers: std::collections::BTreeSet<Router> =
            chosen.iter().map(|&i| routes[i].router).collect();
        if routers.len() > 1 {
            m.ambiguities.push(Ambiguity::RouterConflict);
        }
        let extra: Vec<String> = sound
            .iter()
            .filter(|i| !chosen.contains(i))
            .map(|&i| routes[i].id.clone())
            .collect();
        if !static_only && !extra.is_empty() && !pat.open_tail && !pat.is_concrete() {
            m.ambiguities.push(Ambiguity::ParamMayEqualStatic { extra });
        }
        m.routes
            .extend(chosen.iter().map(|&i| routes[i].id.clone()));
        m.candidates
            .extend(sound.iter().map(|&i| routes[i].id.clone()));
        if !chosen.is_empty() {
            m.outcome = Outcome::Routes;
            true
        } else {
            // Only sound candidates (the heuristic says 404 here; keep looking in later
            // layers, but remember the candidates).
            false
        }
    }
}

enum Rw {
    None,
    To(UrlPat),
    Done,
}
