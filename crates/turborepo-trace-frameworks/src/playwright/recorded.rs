//! Recorded visits: which routes of a Next.js build each Playwright spec
//! reached when it ran.
//!
//! Validation tooling: recordings say where specs really went, so that the
//! selection from source can be checked. They are not an input of selection.
//!
//! A recorder writes one JSON file per run of a test: the requests the test
//! made to the app, with the response headers that say which route answered.
//! This module reads a directory of such files, groups them by spec and test,
//! and maps every request to routes of the build the test ran against.
//!
//! # Which recordings count
//!
//! Per test (spec, project, title), the newest recording decides the build;
//! every recording of the test against that build is used, and their requests
//! are united (a test does not make the same requests on every run). A test
//! whose runs against that build all failed has no usable recording: a failed
//! run stops early, so its requests are incomplete.
//!
//! # Request → route
//!
//! The mapping needs no build output, only the [`Fingerprint`] of the build the
//! recording ran against (it carries the routes manifest's rules and regexes):
//!
//! | Evidence | Route |
//! |---|---|
//! | `x-matched-path`, `x-nextjs-matched-path` response header | the named route |
//! | `x-nextjs-rewritten-path` | the route matching the rewritten path |
//! | `x-middleware-rewrite` | the routes the rewritten path resolves to |
//! | a 3xx answer and a redirect rule that matches | none (the rule answered; rule changes are tested against the path) |
//! | any other 3xx | the route matching the path (a redirect is a visit of the redirecting route) |
//! | otherwise | Next's order: `beforeFiles` rewrites, `public/` files and static routes, `afterFiles` rewrites, dynamic routes, `fallback` rewrites, the not-found route |
//!
//! Rule conditions (`has`, `missing`) are evaluated when the recording has the
//! value (the `next-url` header, the host, the names of the query parameters).
//! Otherwise both outcomes are kept, so the result is a superset of the routes
//! that answered. A main-frame URL change (`nav`) is matched both as a hard and
//! as a soft navigation, because a soft navigation served from the prefetch
//! cache makes no request that would tell them apart. Prefetches are not
//! visits. Requests for `/_next/static/` files are not mapped to routes,
//! but their paths count when routing rules or the middleware change.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    fsx,
    nextjs::{
        build::fingerprint::Fingerprint,
        p2r::{PathPattern, compile_destination},
    },
    pattern::JsRegex,
};

/// The recording format this module reads.
pub const SCHEMA_VERSION: u32 = 1;

/// What kind of request a recorded entry is.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestKind {
    /// A page or frame navigation request.
    Document,
    /// A main-frame URL change (also a soft navigation that made no request).
    Nav,
    /// An App Router navigation or refresh request.
    Rsc,
    /// A Pages Router data request.
    Data,
    /// A server action call.
    Action,
    /// A call through Playwright's request API.
    Api,
    /// Another request a page made.
    Fetch,
    /// An image optimizer request.
    Image,
    /// An App Router prefetch (not a visit).
    Prefetch,
    /// A file under `/_next/static/`.
    Asset,
    /// A kind a newer recorder wrote.
    #[serde(other)]
    Other,
}

fn one() -> u32 {
    1
}

/// One recorded request (identical requests are merged).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Request {
    /// The kind.
    pub kind: RequestKind,
    /// The method.
    #[serde(default)]
    pub method: String,
    /// The URL path.
    pub path: String,
    /// The response status, if a response was seen.
    #[serde(default)]
    pub status: Option<u16>,
    /// The host, when it is not the base URL's.
    #[serde(default)]
    pub host: Option<String>,
    /// The names of the query parameters.
    #[serde(default)]
    pub query: Vec<String>,
    /// Request headers (`next-url`, `next-action`, ...).
    #[serde(default)]
    pub req: BTreeMap<String, String>,
    /// Response headers (`location`, `x-middleware-*`, `x-nextjs-*`, ...).
    #[serde(default)]
    pub res: BTreeMap<String, String>,
    /// `image`: the image.
    #[serde(default)]
    pub target: Option<String>,
    /// `nav`: the path before.
    #[serde(default)]
    pub from: Option<String>,
    /// Made in a hook or a worker fixture.
    #[serde(default)]
    pub hook: bool,
    /// How many times.
    #[serde(default = "one")]
    pub n: u32,
}

/// One file: the requests of one run of one test.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recording {
    /// The format version.
    pub schema: u32,
    /// The spec file, relative to the repository root.
    pub spec: String,
    /// The git blob id of the spec file when the test ran.
    #[serde(default)]
    pub spec_hash: Option<String>,
    /// The describe titles and the test title.
    #[serde(default)]
    pub title: Vec<String>,
    /// The Playwright project.
    #[serde(default)]
    pub project: String,
    /// The retry number.
    #[serde(default)]
    pub retry: u32,
    /// The outcome.
    #[serde(default)]
    pub status: Option<String>,
    /// The expected outcome.
    #[serde(default)]
    pub expected_status: Option<String>,
    /// When the test started (ISO 8601).
    #[serde(default)]
    pub started_at: String,
    /// The base URL.
    #[serde(default, rename = "baseURL")]
    pub base_url: Option<String>,
    /// The Next.js build id the test ran against.
    #[serde(default)]
    pub build: Option<String>,
    /// The commit of the checkout.
    #[serde(default)]
    pub commit: Option<String>,
    /// The requests.
    #[serde(default)]
    pub requests: Vec<Request>,
    /// The file it was read from.
    #[serde(skip)]
    pub file: PathBuf,
}

impl Recording {
    /// Whether the run ended as expected (its request list is complete).
    pub fn as_expected(&self) -> bool {
        match (&self.status, &self.expected_status) {
            (Some(s), Some(e)) => s == e,
            (Some(s), None) => s == "passed" || s == "skipped",
            (None, _) => false,
        }
    }

    /// The host of the base URL, without the port.
    pub fn base_host(&self) -> Option<&str> {
        let u = self.base_url.as_deref()?;
        let rest = u.split_once("://").map_or(u, |(_, r)| r);
        let host = rest.split(['/', '?', '#']).next()?;
        Some(host.rsplit_once(':').map_or(host, |(h, _)| h))
    }
}

/// What a visits directory holds.
#[derive(Clone, Debug, Default)]
pub struct Loaded {
    /// The recordings.
    pub recordings: Vec<Recording>,
    /// Build fingerprints found next to them (`next fingerprint` output).
    pub fingerprints: Vec<PathBuf>,
    /// Files that are recordings but could not be used, with the reason.
    pub problems: Vec<(PathBuf, String)>,
}

#[derive(Deserialize)]
struct Probe {
    schema: Option<u32>,
    version: Option<u32>,
    requests: Option<serde::de::IgnoredAny>,
    routes: Option<serde::de::IgnoredAny>,
}

/// Reads every recording under `dir` (recursively). Other JSON files are
/// ignored, except build fingerprints, which are listed.
pub fn load_dir(dir: &Path) -> Result<Loaded, String> {
    if !fsx::is_dir(dir) {
        return Err(format!("{}: not a directory", dir.display()));
    }
    let mut files: Vec<PathBuf> = Vec::new();
    fsx::walk_files(dir, &mut |p| {
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.ends_with(".json") && !name.starts_with('.') {
            files.push(p.to_path_buf());
        }
    });
    files.sort();
    let mut out = Loaded::default();
    for f in files {
        let Ok(bytes) = fsx::read(&f) else {
            out.problems.push((f, "unreadable".to_string()));
            continue;
        };
        let Ok(probe) = serde_json::from_slice::<Probe>(&bytes) else {
            continue;
        };
        match probe {
            Probe {
                schema: Some(v),
                requests: Some(_),
                ..
            } => {
                if v > SCHEMA_VERSION {
                    out.problems.push((
                        f,
                        format!(
                            "recording format {v} is newer than this version reads \
                             ({SCHEMA_VERSION})"
                        ),
                    ));
                    continue;
                }
                match serde_json::from_slice::<Recording>(&bytes) {
                    Ok(mut r) => {
                        r.file = f;
                        out.recordings.push(r);
                    }
                    Err(e) => out.problems.push((f, format!("not a recording: {e}"))),
                }
            }
            Probe {
                version: Some(_),
                routes: Some(_),
                schema: None,
                ..
            } => out.fingerprints.push(f),
            _ => {}
        }
    }
    Ok(out)
}

/// The recordings of one test that count.
#[derive(Clone, Debug)]
pub struct TestRuns<'a> {
    /// The project.
    pub project: &'a str,
    /// The title path.
    pub title: &'a [String],
    /// The build of the newest recording.
    pub build: Option<&'a str>,
    /// The recordings against that build, oldest first.
    pub runs: Vec<&'a Recording>,
    /// Recordings against other (older) builds.
    pub superseded: usize,
}

impl TestRuns<'_> {
    /// Whether a run against the build ended as expected.
    pub fn usable(&self) -> bool {
        self.runs.iter().any(|r| r.as_expected())
    }

    /// The title as one line.
    pub fn name(&self) -> String {
        self.title.join(" › ")
    }
}

/// Groups recordings by spec, then by test.
pub fn by_spec(recordings: &[Recording]) -> BTreeMap<&str, Vec<TestRuns<'_>>> {
    let mut tests: BTreeMap<(&str, &str, &[String]), Vec<&Recording>> = BTreeMap::new();
    for r in recordings {
        tests
            .entry((r.spec.as_str(), r.project.as_str(), r.title.as_slice()))
            .or_default()
            .push(r);
    }
    let mut out: BTreeMap<&str, Vec<TestRuns<'_>>> = BTreeMap::new();
    for ((spec, project, title), mut runs) in tests {
        runs.sort_by(|a, b| (&a.started_at, a.retry).cmp(&(&b.started_at, b.retry)));
        let build = runs.last().and_then(|r| r.build.as_deref());
        let total = runs.len();
        runs.retain(|r| r.build.as_deref() == build);
        out.entry(spec).or_default().push(TestRuns {
            project,
            title,
            build,
            superseded: total - runs.len(),
            runs,
        });
    }
    out
}

/// Three-valued condition result.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Tri {
    Yes,
    No,
    Maybe,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Phase {
    Redirect,
    Before,
    After,
    Fallback,
}

#[derive(Debug)]
struct Cond {
    kind: String,
    key: String,
    value: Option<JsRegex>,
}

#[derive(Debug)]
struct Rule {
    phase: Phase,
    source: String,
    regex: JsRegex,
    destination: Option<String>,
    /// Parameter names of the source in group order; `None` if the source could
    /// not be read.
    names: Option<Vec<String>>,
    has: Vec<Cond>,
    missing: Vec<Cond>,
}

fn conds(v: Option<&Value>) -> Vec<Cond> {
    v.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|c| {
            let s = |k: &str| c.get(k).and_then(Value::as_str).map(str::to_string);
            Cond {
                kind: s("type").unwrap_or_default(),
                key: s("key").unwrap_or_default().to_ascii_lowercase(),
                // Next matches the whole value.
                value: s("value").map(|v| JsRegex::new(format!("^(?:{v})$"), "")),
            }
        })
        .collect()
}

/// What a request resolved to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Resolution {
    /// The routes that may have answered (pathnames of the fingerprint).
    pub routes: BTreeSet<String>,
    /// The `public/` file that answered (relative to `public/`).
    pub public: Option<String>,
    /// No route matched: the not-found route answered (it is in `routes`).
    pub not_found: bool,
    /// A rewrite sent the request to another origin.
    pub external: Option<String>,
    /// A redirect rule answered (its source).
    pub redirect_rule: Option<String>,
    /// The request could not be mapped: the reason. The caller must not skip
    /// the spec.
    pub unmapped: Option<String>,
}

/// Maps requests to routes of one build.
#[derive(Debug)]
pub struct RouteMapper<'a> {
    fp: &'a Fingerprint,
    statics: Vec<(&'a str, JsRegex)>,
    dynamics: Vec<(&'a str, JsRegex)>,
    rules: Vec<Rule>,
    locales: Vec<String>,
    default_locale: Option<String>,
    not_found: Vec<String>,
    /// A regex of the manifest that no engine runs: no request can be mapped
    /// soundly.
    broken: Option<String>,
    /// The middleware matchers: `None` without middleware, `Some(None)` when it
    /// runs on every path (or a matcher cannot be read).
    middleware: Option<Option<Vec<JsRegex>>>,
}

const MAX_REWRITES: usize = 4;

impl<'a> RouteMapper<'a> {
    /// A mapper for the build `fp` describes.
    pub fn new(fp: &'a Fingerprint) -> RouteMapper<'a> {
        let mut statics = Vec::new();
        let mut dynamics = Vec::new();
        let mut broken: Option<String> = None;
        // Rules match case-insensitively unless `caseSensitiveRoutes` is set; routes
        // always match exactly.
        let rule_flags = if fp.case_sensitive { "" } else { "i" };
        for r in &fp.route_regexes {
            let re = JsRegex::new(r.regex.clone(), "");
            if !re.is_supported() {
                broken.get_or_insert_with(|| {
                    format!("the regex of route {} is not supported", r.page)
                });
            }
            if r.page.contains('[') {
                dynamics.push((r.page.as_str(), re));
            } else {
                statics.push((r.page.as_str(), re));
            }
        }
        let rules: Vec<Rule> = fp
            .rules
            .iter()
            .filter_map(|r| {
                let phase = match r.phase.as_str() {
                    "redirect" => Phase::Redirect,
                    "rewrite:beforeFiles" => Phase::Before,
                    "rewrite:afterFiles" => Phase::After,
                    "rewrite:fallback" => Phase::Fallback,
                    _ => return None,
                };
                Some(Rule {
                    phase,
                    source: r.source.clone(),
                    regex: JsRegex::new(r.regex.clone(), rule_flags),
                    destination: r.destination.clone(),
                    names: PathPattern::new(&r.source).ok().map(|p| p.names().to_vec()),
                    has: conds(r.raw.get("has")),
                    missing: conds(r.raw.get("missing")),
                })
            })
            .collect();
        if let Some(r) = rules.iter().find(|r| !r.regex.is_supported()) {
            broken
                .get_or_insert_with(|| format!("the regex of rule {} is not supported", r.source));
        }
        let locales: Vec<String> = fp
            .i18n
            .get("locales")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|l| l.as_str().map(str::to_ascii_lowercase))
            .collect();
        let default_locale = fp
            .i18n
            .get("defaultLocale")
            .and_then(Value::as_str)
            .map(str::to_string);
        let not_found = ["/_not-found", "/404", "/_error"]
            .iter()
            .filter(|r| fp.routes.contains_key(**r))
            .map(|r| (*r).to_string())
            .collect();
        RouteMapper {
            fp,
            statics,
            dynamics,
            rules,
            locales,
            default_locale,
            not_found,
            broken,
            middleware: fp.global.contains_key("middleware").then(|| {
                let list = fp.middleware_matchers.as_ref()?.as_array()?;
                list.iter()
                    .map(|m| {
                        let re = JsRegex::new(m.get("regexp")?.as_str()?, "");
                        re.is_supported().then_some(re)
                    })
                    .collect()
            }),
        }
    }

    /// Whether the middleware may run on `path` (and so may rewrite it).
    pub fn middleware_runs_on(&self, path: &str) -> bool {
        match &self.middleware {
            None => false,
            Some(None) => true,
            Some(Some(list)) => list.iter().any(|re| re.is_match(path)),
        }
    }

    /// The fingerprint.
    pub fn fingerprint(&self) -> &'a Fingerprint {
        self.fp
    }

    /// `path` as routes see it: without the base path, the locale, an `.rsc`
    /// suffix and a trailing slash. `None` if the path is outside the base
    /// path.
    pub fn route_path(&self, path: &str) -> Option<String> {
        let path = path.split(['?', '#']).next().unwrap_or(path);
        let mut p = path;
        let base = self.fp.base_path.as_str();
        if !base.is_empty() {
            p = match p.strip_prefix(base) {
                Some("") => "/",
                Some(rest) if rest.starts_with('/') => rest,
                _ => return None,
            };
        }
        let mut p = p.strip_suffix(".rsc").unwrap_or(p).to_string();
        if !self.locales.is_empty()
            && let Some(first) = p.trim_start_matches('/').split('/').next()
            && self.locales.contains(&first.to_ascii_lowercase())
        {
            p = p[1 + first.len()..].to_string();
        }
        while p.len() > 1 && p.ends_with('/') {
            p.pop();
        }
        if p.is_empty() {
            p.push('/');
        }
        Some(p)
    }

    /// The path of a Pages Router data request: `/_next/data/<build>/a/b.json`
    /// → `/a/b`.
    pub fn data_path(&self, path: &str) -> String {
        let Some(at) = path.find("/_next/data/") else {
            return path.to_string();
        };
        let rest = &path[at + "/_next/data/".len()..];
        let rest = rest.split_once('/').map_or("", |(_, r)| r);
        let rest = rest.strip_suffix(".json").unwrap_or(rest);
        let rest = if rest == "index" { "" } else { rest };
        format!("{}/{rest}", &path[..at])
    }

    fn rule_matches(&self, rule: &Rule, path: &str) -> Option<crate::pattern::RegexMatch> {
        if let Some(m) = rule.regex.find_at(path, 0) {
            return Some(m);
        }
        // With i18n the rules carry a locale group; a path without a locale has the
        // default.
        let d = self.default_locale.as_deref()?;
        let first = path.trim_start_matches('/').split('/').next().unwrap_or("");
        if self.locales.contains(&first.to_ascii_lowercase()) {
            return None;
        }
        rule.regex.find_at(&format!("/{d}{path}"), 0)
    }

    fn cond(&self, c: &Cond, r: &Request, host: Option<&str>) -> Tri {
        let value = |v: Option<&str>| match (v, &c.value) {
            (None, _) => Tri::No,
            (Some(_), None) => Tri::Yes,
            (Some(_), Some(re)) if !re.is_supported() => Tri::Maybe,
            (Some(v), Some(re)) => {
                if re.is_match(v) {
                    Tri::Yes
                } else {
                    Tri::No
                }
            }
        };
        match c.kind.as_str() {
            "header" if c.key == "next-url" => {
                if r.kind == RequestKind::Nav {
                    // Hard or soft: not known.
                    Tri::Maybe
                } else {
                    value(r.req.get("next-url").map(String::as_str))
                }
            }
            "header" if r.req.contains_key(&c.key) => value(r.req.get(&c.key).map(String::as_str)),
            "host" => match r.host.as_deref().or(host) {
                Some(h) => value(Some(h.rsplit_once(':').map_or(h, |(h, _)| h))),
                None => Tri::Maybe,
            },
            "query" => {
                if !r.query.iter().any(|q| q.eq_ignore_ascii_case(&c.key)) {
                    Tri::No
                } else if c.value.is_none() {
                    Tri::Yes
                } else {
                    Tri::Maybe
                }
            }
            _ => Tri::Maybe,
        }
    }

    fn applies(&self, rule: &Rule, r: &Request, host: Option<&str>) -> Tri {
        let mut out = Tri::Yes;
        for c in &rule.has {
            match self.cond(c, r, host) {
                Tri::No => return Tri::No,
                Tri::Maybe => out = Tri::Maybe,
                Tri::Yes => {}
            }
        }
        for c in &rule.missing {
            match self.cond(c, r, host) {
                Tri::Yes => return Tri::No,
                Tri::Maybe => out = Tri::Maybe,
                Tri::No => {}
            }
        }
        out
    }

    /// The destination of a rewrite for a matched path: `Ok(path)`, or
    /// `Err(origin)` for another origin (`Err("")` if it could not be
    /// computed).
    fn destination(&self, rule: &Rule, m: &crate::pattern::RegexMatch) -> Result<String, String> {
        let dest = rule.destination.as_deref().ok_or_else(String::new)?;
        let filled = if dest.contains(':') {
            let names = rule.names.as_ref().ok_or_else(String::new)?;
            let params: Vec<(String, Option<String>)> = names
                .iter()
                .enumerate()
                .map(|(i, n)| (n.clone(), m.groups.get(i + 1).cloned().flatten()))
                .collect();
            compile_destination(dest, &params).ok_or_else(String::new)?
        } else {
            dest.to_string()
        };
        if let Some((scheme, rest)) = filled.split_once("://")
            && (scheme == "http" || scheme == "https")
        {
            return Err(rest.split('/').next().unwrap_or(rest).to_string());
        }
        Ok(filled
            .split(['?', '#'])
            .next()
            .unwrap_or(&filled)
            .to_string())
    }

    fn first(&self, list: &[(&'a str, JsRegex)], path: &str) -> Option<&'a str> {
        list.iter()
            .find(|(_, re)| re.is_match(path))
            .map(|(page, _)| *page)
    }

    fn insert_route(&self, page: &str, out: &mut Resolution) {
        if self.fp.routes.contains_key(page) {
            out.routes.insert(page.to_string());
        } else {
            out.unmapped = Some(format!(
                "matched {page}, which the fingerprint has no route for"
            ));
        }
    }

    /// Files, then `afterFiles` rewrites, dynamic routes, `fallback` rewrites,
    /// not found.
    fn files(
        &self,
        path: &str,
        r: &Request,
        host: Option<&str>,
        depth: usize,
        out: &mut Resolution,
    ) {
        let Some(p) = self.route_path(path) else {
            self.missing(out);
            return;
        };
        let file = p.trim_start_matches('/');
        if !file.is_empty() && self.fp.public.contains_key(file) {
            out.public = Some(file.to_string());
            return;
        }
        if let Some(page) = self.first(&self.statics, &p) {
            self.insert_route(page, out);
            return;
        }
        if self.rewrites(Phase::After, path, r, host, depth, out) {
            return;
        }
        if let Some(page) = self.first(&self.dynamics, &p) {
            self.insert_route(page, out);
            return;
        }
        if self.rewrites(Phase::Fallback, path, r, host, depth, out) {
            return;
        }
        self.missing(out);
    }

    fn missing(&self, out: &mut Resolution) {
        out.not_found = true;
        out.routes.extend(self.not_found.iter().cloned());
    }

    /// Applies the rewrites of `phase` to `path`. True if one certainly
    /// applied.
    fn rewrites(
        &self,
        phase: Phase,
        path: &str,
        r: &Request,
        host: Option<&str>,
        depth: usize,
        out: &mut Resolution,
    ) -> bool {
        for rule in self.rules.iter().filter(|x| x.phase == phase) {
            let Some(m) = self.rule_matches(rule, path) else {
                continue;
            };
            let sure = match self.applies(rule, r, host) {
                Tri::No => continue,
                Tri::Yes => true,
                Tri::Maybe => false,
            };
            match self.destination(rule, &m) {
                Ok(dest) if depth < MAX_REWRITES => self.files(&dest, r, host, depth + 1, out),
                Ok(_) => out.unmapped = Some(format!("rewrites of {path} do not end")),
                Err(origin) if !origin.is_empty() => out.external = Some(origin),
                Err(_) => {
                    out.unmapped = Some(format!(
                        "the destination of the rewrite {} could not be computed",
                        rule.source
                    ));
                }
            }
            if sure {
                return true;
            }
        }
        false
    }

    /// The routes `r` may have reached. `host` is the host of the recording's
    /// base URL.
    pub fn resolve(&self, r: &Request, host: Option<&str>) -> Resolution {
        let mut out = Resolution {
            unmapped: self.broken.clone(),
            ..Resolution::default()
        };
        let mut path = match r.kind {
            RequestKind::Asset | RequestKind::Prefetch | RequestKind::Other => {
                return Resolution::default();
            }
            RequestKind::Image => match r.target.as_deref() {
                Some(t) if t.starts_with('/') => t.to_string(),
                _ => return Resolution::default(),
            },
            RequestKind::Data => self.data_path(&r.path),
            _ => r.path.clone(),
        };
        // The server names the route.
        for h in ["x-matched-path", "x-nextjs-matched-path"] {
            if let Some(v) = r.res.get(h) {
                let page = v.strip_suffix(".rsc").unwrap_or(v);
                let page = if page == "/index" { "/" } else { page };
                if self.fp.routes.contains_key(page) {
                    out.routes.insert(page.to_string());
                    return out;
                }
            }
        }
        let redirected = matches!(r.status, Some(301 | 302 | 303 | 307 | 308));
        if !redirected {
            if let Some(v) = r.res.get("x-nextjs-rewritten-path")
                && let Some(p) = self.route_path(v)
                && let Some(page) = self
                    .first(&self.statics, &p)
                    .or_else(|| self.first(&self.dynamics, &p))
            {
                self.insert_route(page, &mut out);
                return out;
            }
            if let Some(v) = r.res.get("x-middleware-rewrite") {
                match v.split_once("://") {
                    Some((_, rest)) => {
                        let (h, p) = rest.split_once('/').unwrap_or((rest, ""));
                        let own = r.host.as_deref().or(host);
                        let bare = |x: &str| x.rsplit_once(':').map_or(x, |(a, _)| a).to_string();
                        if own.is_some_and(|o| bare(o) != bare(h)) {
                            out.external = Some(h.to_string());
                            return out;
                        }
                        path = format!("/{p}");
                    }
                    None => path = v.clone(),
                }
                path = path.split(['?', '#']).next().unwrap_or(&path).to_string();
            }
        }
        // Redirect rules run first. Without a status (a redirect the request API
        // followed, a request without a response) a matching rule may or may
        // not have answered.
        if (redirected || r.status.is_none()) && r.kind != RequestKind::Nav {
            for rule in self.rules.iter().filter(|x| x.phase == Phase::Redirect) {
                if self.rule_matches(rule, &path).is_none() {
                    continue;
                }
                match self.applies(rule, r, host) {
                    Tri::No => {}
                    Tri::Yes if redirected => {
                        out.redirect_rule = Some(rule.source.clone());
                        return out;
                    }
                    _ => {
                        out.redirect_rule = Some(rule.source.clone());
                        break;
                    }
                }
            }
        }
        // `beforeFiles` rewrites, then the files.
        let mut candidates: Vec<String> = vec![path.clone()];
        for rule in self.rules.iter().filter(|x| x.phase == Phase::Before) {
            let Some(m) = self.rule_matches(rule, &path) else {
                continue;
            };
            let sure = match self.applies(rule, r, host) {
                Tri::No => continue,
                Tri::Yes => true,
                Tri::Maybe => false,
            };
            match self.destination(rule, &m) {
                Ok(dest) => {
                    if sure {
                        candidates.clear();
                    }
                    candidates.push(dest);
                }
                Err(origin) if !origin.is_empty() => {
                    out.external = Some(origin);
                    if sure {
                        candidates.clear();
                    }
                }
                Err(_) => {
                    out.unmapped = Some(format!(
                        "the destination of the rewrite {} could not be computed",
                        rule.source
                    ));
                }
            }
            if sure {
                break;
            }
        }
        for c in candidates {
            self.files(&c, r, host, 0, &mut out);
        }
        out
    }
}

/// The path a server action's response redirects to (`x-action-redirect:
/// /path;push`).
fn action_redirect(r: &Request) -> Option<String> {
    let v = r.res.get("x-action-redirect")?;
    let url = v.split(';').next()?;
    let path = match url.split_once("://") {
        Some((_, rest)) => rest.find('/').map_or("/", |i| &rest[i..]),
        None => url,
    };
    let path = path.split(['?', '#']).next()?;
    path.starts_with('/').then(|| path.to_string())
}

/// The strongest evidence that a spec reached a route.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Evidence {
    /// The kind of the request.
    pub kind: RequestKind,
    /// The method.
    pub method: String,
    /// The requested path.
    pub path: String,
    /// A test that made the request.
    pub test: String,
}

/// What the recordings of a spec against one build reached.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Visits {
    /// Routes → the evidence.
    pub routes: BTreeMap<String, Evidence>,
    /// Every requested path, of every kind (rules and middleware matchers are
    /// tested against them).
    pub paths: BTreeSet<String>,
    /// `public/` files that answered → a requested path.
    pub public: BTreeMap<String, String>,
    /// Paths no route matched.
    pub not_found: BTreeSet<String>,
    /// Origins rewrites sent requests to.
    pub external: BTreeSet<String>,
    /// Requests that could not be mapped (path, reason).
    pub unmapped: Vec<(String, String)>,
    /// The number of recordings read.
    pub recordings: usize,
    /// The number of requests (after merging identical ones).
    pub requests: usize,
}

impl Visits {
    /// Adds the requests of `rec`.
    pub fn add(&mut self, mapper: &RouteMapper<'_>, rec: &Recording) {
        self.recordings += 1;
        let host = rec.base_host();
        let test = rec.title.join(" › ");
        // What the server answered for each path. A URL change carries no headers of
        // its own: it is resolved with the answers for its path (the document,
        // the RSC payload or the prefetch the router used), which say where a
        // middleware sent it.
        let mut answers: BTreeMap<String, Vec<&Request>> = BTreeMap::new();
        for r in &rec.requests {
            if matches!(
                r.kind,
                RequestKind::Document
                    | RequestKind::Rsc
                    | RequestKind::Prefetch
                    | RequestKind::Data
            ) && r.status.is_some_and(|s| !(300..400).contains(&s))
            {
                // A Pages Router navigation asks for the data of the page it goes to.
                let path = if r.kind == RequestKind::Data {
                    mapper.data_path(&r.path)
                } else {
                    r.path.clone()
                };
                answers.entry(path).or_default().push(r);
            }
            // A server action that redirects answers with the page it redirects to: the
            // rewrite headers of that response are the target's.
            if let Some(target) = action_redirect(r) {
                answers.entry(target).or_default().push(r);
            }
        }
        for r in &rec.requests {
            self.requests += 1;
            self.paths.insert(r.path.clone());
            if let Some(t) = &r.target
                && t.starts_with('/')
            {
                self.paths.insert(t.clone());
            }
            let mut res = Resolution::default();
            let mut merge = |x: Resolution| {
                res.routes.extend(x.routes);
                res.public = res.public.take().or(x.public);
                res.not_found |= x.not_found;
                res.external = res.external.take().or(x.external);
                res.unmapped = res.unmapped.take().or(x.unmapped);
            };
            match (r.kind, answers.get(&r.path)) {
                (RequestKind::Nav, Some(list)) => {
                    for a in list {
                        let mut n = r.clone();
                        n.res.clone_from(&a.res);
                        merge(mapper.resolve(&n, host));
                    }
                }
                (RequestKind::Nav, None) if mapper.middleware_runs_on(&r.path) => {
                    merge(Resolution {
                        unmapped: Some(
                            "the URL changed to it and no response says where the middleware sent \
                             it"
                            .to_string(),
                        ),
                        ..Resolution::default()
                    });
                }
                // The action ran on the route of its own path, which other answers for
                // that path name; its own headers name the redirect target.
                (_, own) if action_redirect(r).is_some() => {
                    let own: Vec<&&Request> = own
                        .into_iter()
                        .flatten()
                        .filter(|a| !std::ptr::eq(**a, r))
                        .collect();
                    if !own.is_empty() {
                        for a in own {
                            let mut n = r.clone();
                            n.res.clone_from(&a.res);
                            merge(mapper.resolve(&n, host));
                        }
                    } else if mapper.middleware_runs_on(&r.path) {
                        merge(Resolution {
                            unmapped: Some(
                                "a server action redirected from it and no response says where \
                                 the middleware sent it"
                                    .to_string(),
                            ),
                            ..Resolution::default()
                        });
                    } else {
                        let mut n = r.clone();
                        n.res.clear();
                        merge(mapper.resolve(&n, host));
                    }
                }
                _ => merge(mapper.resolve(r, host)),
            }
            for route in res.routes {
                let e = Evidence {
                    kind: r.kind,
                    method: r.method.clone(),
                    path: r.path.clone(),
                    test: test.clone(),
                };
                match self.routes.get(&route) {
                    Some(old) if old.kind <= e.kind => {}
                    _ => {
                        self.routes.insert(route, e);
                    }
                }
            }
            if let Some(f) = res.public {
                self.public.entry(f).or_insert_with(|| r.path.clone());
            }
            if res.not_found {
                self.not_found.insert(r.path.clone());
            }
            if let Some(o) = res.external {
                self.external.insert(o);
            }
            if let Some(why) = res.unmapped {
                self.unmapped.push((r.path.clone(), why));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fp(extra: Value) -> Fingerprint {
        let mut v = serde_json::json!({
            "version": 3,
            "build_id": "b1",
            "bundler": "turbopack",
            "per_module": true,
            "routes": {},
            "global": {},
            "rules": [
                {"phase": "redirect", "source": "/:path+/", "regex": "^(?:/((?:[^/]+?)(?:/(?:[^/]+?))*))/$", "destination": "/:path+", "internal": true, "raw": {}},
                {"phase": "redirect", "source": "/old-about", "regex": "^(?!/_next)/old-about(?:/)?$", "destination": "/about", "internal": false, "raw": {}},
                {"phase": "rewrite:beforeFiles", "source": "/photos/photo/:nxtPid", "regex": "^/photos/photo(?:/([^/]+?))(?:/)?$", "destination": "/photos/(.)photo/:nxtPid", "internal": false,
                 "raw": {"has": [{"type": "header", "key": "next-url", "value": "/photos(?:/.*)?"}]}},
                {"phase": "rewrite:afterFiles", "source": "/p/:slug", "regex": "^/p(?:/([^/]+?))(?:/)?$", "destination": "/blog/:slug", "internal": false, "raw": {}},
                {"phase": "rewrite:afterFiles", "source": "/v1/:path*", "regex": "^/v1(?:/((?:[^/]+?)(?:/(?:[^/]+?))*))?(?:/)?$", "destination": "/api/v1/:path*", "internal": false,
                 "raw": {"has": [{"type": "host", "value": "api.example.com"}]}},
                {"phase": "rewrite:fallback", "source": "/ext/:path*", "regex": "^/ext(?:/((?:[^/]+?)(?:/(?:[^/]+?))*))?(?:/)?$", "destination": "https://other.example.com/:path*", "internal": false, "raw": {}}
            ],
            "route_regexes": [
                {"page": "/", "regex": "^/(?:/)?$"},
                {"page": "/_not-found", "regex": "^/_not\\-found(?:/)?$"},
                {"page": "/about", "regex": "^/about(?:/)?$"},
                {"page": "/api/hello", "regex": "^/api/hello(?:/)?$"},
                {"page": "/blog/[slug]", "regex": "^/blog/([^/]+?)(?:/)?$"},
                {"page": "/api/v1/[...path]", "regex": "^/api/v1/(.+?)(?:/)?$"},
                {"page": "/legacy/[id]", "regex": "^/legacy/([^/]+?)(?:/)?$"},
                {"page": "/photos/(.)photo/[id]", "regex": "^/photos/\\(\\.\\)photo/([^/]+?)(?:/)?$"},
                {"page": "/photos/photo/[id]", "regex": "^/photos/photo/([^/]+?)(?:/)?$"}
            ],
            "public": {"logo.png": "h1"}
        });
        let route = serde_json::json!({
            "key": "k", "router": "app", "kind": "page",
            "server": {"hash": "s", "files": 1}, "client": {"hash": "c", "files": 1},
            "css": {"hash": "x", "files": 0}, "prerender": {"hash": "p", "files": 0},
            "external": "e", "own": "o", "effective": "o"
        });
        let pages: Vec<String> = v["route_regexes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["page"].as_str().unwrap().to_string())
            .collect();
        for p in pages {
            v["routes"][p] = route.clone();
        }
        if let Value::Object(m) = extra {
            for (k, x) in m {
                v[k] = x;
            }
        }
        serde_json::from_value(v).unwrap()
    }

    fn req(kind: RequestKind, path: &str) -> Request {
        Request {
            kind,
            method: "GET".into(),
            path: path.into(),
            status: Some(200),
            host: None,
            query: Vec::new(),
            req: BTreeMap::new(),
            res: BTreeMap::new(),
            target: None,
            from: None,
            hook: false,
            n: 1,
        }
    }

    fn routes(m: &RouteMapper<'_>, r: &Request) -> Vec<String> {
        m.resolve(r, Some("localhost")).routes.into_iter().collect()
    }

    #[test]
    fn static_dynamic_and_not_found() {
        let f = fp(Value::Null);
        let m = RouteMapper::new(&f);
        assert_eq!(
            routes(&m, &req(RequestKind::Document, "/about")),
            ["/about"]
        );
        assert_eq!(routes(&m, &req(RequestKind::Rsc, "/about/")), ["/about"]);
        assert_eq!(
            routes(&m, &req(RequestKind::Document, "/blog/hello")),
            ["/blog/[slug]"]
        );
        let r = m.resolve(&req(RequestKind::Document, "/nope/x"), None);
        assert!(r.not_found);
        assert_eq!(r.routes.into_iter().collect::<Vec<_>>(), ["/_not-found"]);
        // Prefetches and build assets are not visits.
        assert!(routes(&m, &req(RequestKind::Prefetch, "/about")).is_empty());
        assert!(routes(&m, &req(RequestKind::Asset, "/_next/static/chunks/a.js")).is_empty());
    }

    #[test]
    fn public_files_and_images() {
        let f = fp(Value::Null);
        let m = RouteMapper::new(&f);
        let r = m.resolve(&req(RequestKind::Fetch, "/logo.png"), None);
        assert_eq!(r.public.as_deref(), Some("logo.png"));
        assert!(r.routes.is_empty());
        let mut img = req(RequestKind::Image, "/_next/image");
        img.target = Some("/logo.png".into());
        assert_eq!(m.resolve(&img, None).public.as_deref(), Some("logo.png"));
        img.target = Some("https://cdn.example.com/x.png".into());
        assert_eq!(m.resolve(&img, None), Resolution::default());
    }

    #[test]
    fn rewrites_follow_nexts_order() {
        let f = fp(Value::Null);
        let m = RouteMapper::new(&f);
        // afterFiles: the destination's route.
        assert_eq!(
            routes(&m, &req(RequestKind::Document, "/p/hello")),
            ["/blog/[slug]"]
        );
        // A host condition that does not hold: the rule is skipped (not found).
        assert_eq!(
            routes(&m, &req(RequestKind::Api, "/v1/polls")),
            ["/_not-found"]
        );
        let mut api = req(RequestKind::Api, "/v1/polls");
        api.host = Some("api.example.com:3000".into());
        assert_eq!(routes(&m, &api), ["/api/v1/[...path]"]);
        // A fallback rewrite to another origin.
        let r = m.resolve(&req(RequestKind::Fetch, "/ext/a/b"), None);
        assert_eq!(r.external.as_deref(), Some("other.example.com"));
        assert!(r.routes.is_empty());
    }

    #[test]
    fn interception_depends_on_next_url() {
        let f = fp(Value::Null);
        let m = RouteMapper::new(&f);
        // A hard load: the page.
        assert_eq!(
            routes(&m, &req(RequestKind::Document, "/photos/photo/1")),
            ["/photos/photo/[id]"]
        );
        // A soft navigation from /photos: the intercepting route.
        let mut soft = req(RequestKind::Rsc, "/photos/photo/1");
        soft.req.insert("next-url".into(), "/photos".into());
        assert_eq!(routes(&m, &soft), ["/photos/(.)photo/[id]"]);
        // A URL change alone does not say which: both.
        assert_eq!(
            routes(&m, &req(RequestKind::Nav, "/photos/photo/1")),
            ["/photos/(.)photo/[id]", "/photos/photo/[id]"]
        );
    }

    #[test]
    fn redirects() {
        let f = fp(Value::Null);
        let m = RouteMapper::new(&f);
        // A redirect rule answered: no route.
        let mut r = req(RequestKind::Document, "/old-about");
        r.status = Some(307);
        let res = m.resolve(&r, None);
        assert_eq!(res.redirect_rule.as_deref(), Some("/old-about"));
        assert!(res.routes.is_empty());
        // A 3xx without a rule: the route (or the middleware in front of it)
        // redirected.
        let mut r = req(RequestKind::Document, "/blog/x");
        r.status = Some(307);
        assert_eq!(routes(&m, &r), ["/blog/[slug]"]);
        // The request API followed a redirect: the first hop has no status.
        let mut r = req(RequestKind::Api, "/old-about");
        r.status = None;
        let res = m.resolve(&r, None);
        assert_eq!(res.redirect_rule.as_deref(), Some("/old-about"));
        assert_eq!(res.routes.into_iter().collect::<Vec<_>>(), ["/_not-found"]);
    }

    #[test]
    fn response_headers_name_the_route() {
        let f = fp(Value::Null);
        let m = RouteMapper::new(&f);
        let mut r = req(RequestKind::Document, "/team-a/home");
        r.res.insert(
            "x-middleware-rewrite".into(),
            "http://localhost:3000/about".into(),
        );
        assert_eq!(routes(&m, &r), ["/about"]);
        let mut r = req(RequestKind::Rsc, "/x/y");
        r.res
            .insert("x-nextjs-rewritten-path".into(), "/blog/z".into());
        assert_eq!(routes(&m, &r), ["/blog/[slug]"]);
        let mut r = req(RequestKind::Data, "/_next/data/BUILD/legacy/7.json");
        assert_eq!(routes(&m, &r), ["/legacy/[id]"]);
        r.res
            .insert("x-nextjs-matched-path".into(), "/legacy/[id]".into());
        assert_eq!(routes(&m, &r), ["/legacy/[id]"]);
        let mut r = req(RequestKind::Document, "/whatever");
        r.res.insert("x-matched-path".into(), "/blog/[slug]".into());
        assert_eq!(routes(&m, &r), ["/blog/[slug]"]);
    }

    #[test]
    fn a_url_change_is_resolved_with_the_answer_for_its_path() {
        // A middleware in front of every page rewrites /login to /blog/login.
        let f = fp(serde_json::json!({
            "global": {"middleware": "m"},
            "middleware_matchers": [{"regexp": "^/(?!api).*$", "originalSource": "/((?!api).*)"}]
        }));
        let m = RouteMapper::new(&f);
        let rec = |requests: Vec<Request>| Recording {
            schema: 1,
            spec: "e2e/a.spec.ts".into(),
            spec_hash: None,
            title: vec!["t".into()],
            project: "chromium".into(),
            retry: 0,
            status: Some("passed".into()),
            expected_status: Some("passed".into()),
            started_at: String::new(),
            base_url: Some("http://localhost:3000".into()),
            build: Some("b1".into()),
            commit: None,
            requests,
            file: PathBuf::new(),
        };
        let mut doc = req(RequestKind::Document, "/about");
        doc.res
            .insert("x-middleware-rewrite".into(), "/blog/about".into());
        let mut v = Visits::default();
        v.add(&m, &rec(vec![doc, req(RequestKind::Nav, "/about")]));
        // Not /about: the middleware sent the path elsewhere, and the URL change says
        // nothing else.
        assert_eq!(
            v.routes.keys().cloned().collect::<Vec<_>>(),
            ["/blog/[slug]"]
        );
        assert!(v.unmapped.is_empty());
        // A URL change without any answer for its path: the middleware may have
        // rewritten it.
        let mut v = Visits::default();
        v.add(&m, &rec(vec![req(RequestKind::Nav, "/about")]));
        assert_eq!(v.unmapped.len(), 1);
        // The middleware does not run on /api: the manifest decides.
        let mut v = Visits::default();
        v.add(&m, &rec(vec![req(RequestKind::Nav, "/api/hello")]));
        assert_eq!(v.routes.keys().cloned().collect::<Vec<_>>(), ["/api/hello"]);
        assert!(v.unmapped.is_empty());
        // A server action that redirects: its response answers for the target, and the
        // action itself ran on the route of its own path.
        let mut page = req(RequestKind::Document, "/about");
        page.res
            .insert("x-middleware-rewrite".into(), "/blog/about".into());
        let mut action = req(RequestKind::Action, "/about");
        action.method = "POST".into();
        action
            .res
            .insert("x-action-redirect".into(), "/legacy/9;push".into());
        action
            .res
            .insert("x-nextjs-rewritten-path".into(), "/legacy/9".into());
        let mut v = Visits::default();
        v.add(
            &m,
            &rec(vec![page, action, req(RequestKind::Nav, "/legacy/9")]),
        );
        assert_eq!(
            v.routes.keys().cloned().collect::<Vec<_>>(),
            ["/blog/[slug]", "/legacy/[id]"]
        );
        assert_eq!(v.routes["/blog/[slug]"].kind, RequestKind::Document);
        assert!(v.unmapped.is_empty());
        // A Pages Router navigation: the answer is the data request of the page.
        let mut v = Visits::default();
        v.add(
            &m,
            &rec(vec![
                req(RequestKind::Data, "/_next/data/BUILD/legacy/7.json"),
                req(RequestKind::Nav, "/legacy/7"),
            ]),
        );
        assert_eq!(
            v.routes.keys().cloned().collect::<Vec<_>>(),
            ["/legacy/[id]"]
        );
        assert!(v.unmapped.is_empty());
    }

    #[test]
    fn a_directory_holds_recordings_fingerprints_and_other_files() {
        let d = tempfile::tempdir().unwrap();
        let w = |name: &str, text: &str| std::fs::write(d.path().join(name), text).unwrap();
        w(
            "a.json",
            r#"{"schema":1,"spec":"e2e/a.spec.ts","title":["t"],"status":"passed","expectedStatus":"passed",
                "requests":[{"kind":"document","method":"GET","path":"/","status":200,"n":1},{"kind":"beacon","path":"/x"}]}"#,
        );
        w(
            "fingerprint.json",
            r#"{"version":2,"build_id":"b","routes":{}}"#,
        );
        w("newer.json", r#"{"schema":9,"spec":"x","requests":[]}"#);
        w("other.json", r#"{"hello":1}"#);
        w("broken.json", "{");
        w("notes.txt", "not json");
        let l = load_dir(d.path()).unwrap();
        assert_eq!(l.recordings.len(), 1);
        let kinds: Vec<RequestKind> = l.recordings[0].requests.iter().map(|r| r.kind).collect();
        // A kind a newer recorder wrote is kept as `Other` (and maps to nothing).
        assert_eq!(kinds, [RequestKind::Document, RequestKind::Other]);
        assert!(l.recordings[0].as_expected());
        assert_eq!(l.fingerprints.len(), 1);
        assert!(l.fingerprints[0].ends_with("fingerprint.json"));
        assert_eq!(l.problems.len(), 1);
        assert!(l.problems[0].1.contains("newer"));
        assert!(load_dir(&d.path().join("missing")).is_err());
    }

    #[test]
    fn base_path_and_locales() {
        let f = fp(serde_json::json!({
            "base_path": "/docs",
            "i18n": {"locales": ["en", "fr"], "defaultLocale": "en"}
        }));
        let m = RouteMapper::new(&f);
        assert_eq!(
            routes(&m, &req(RequestKind::Document, "/docs/fr/about")),
            ["/about"]
        );
        assert_eq!(routes(&m, &req(RequestKind::Document, "/docs")), ["/"]);
        // Outside the base path nothing of the app answers.
        assert!(
            m.resolve(&req(RequestKind::Document, "/about"), None)
                .not_found
        );
    }

    #[test]
    fn newest_build_wins_and_runs_unite() {
        let rec = |build: &str, at: &str, status: &str, path: &str| Recording {
            schema: 1,
            spec: "e2e/a.spec.ts".into(),
            spec_hash: None,
            title: vec!["t".into()],
            project: "chromium".into(),
            retry: 0,
            status: Some(status.into()),
            expected_status: Some("passed".into()),
            started_at: at.into(),
            base_url: Some("http://localhost:3000".into()),
            build: Some(build.into()),
            commit: None,
            requests: vec![req(RequestKind::Document, path)],
            file: PathBuf::new(),
        };
        let recs = vec![
            rec("old", "2026-01-01T00:00:00Z", "passed", "/"),
            rec("new", "2026-01-02T00:00:00Z", "failed", "/about"),
            rec("new", "2026-01-02T00:01:00Z", "passed", "/blog/x"),
        ];
        let by = by_spec(&recs);
        let tests = &by["e2e/a.spec.ts"];
        assert_eq!(tests.len(), 1);
        assert_eq!(tests[0].build, Some("new"));
        assert_eq!(tests[0].runs.len(), 2);
        assert_eq!(tests[0].superseded, 1);
        assert!(tests[0].usable());
        let f = fp(Value::Null);
        let m = RouteMapper::new(&f);
        let mut v = Visits::default();
        for r in &tests[0].runs {
            v.add(&m, r);
        }
        assert_eq!(
            v.routes.keys().cloned().collect::<Vec<_>>(),
            ["/about", "/blog/[slug]"]
        );
        assert_eq!(recs[0].base_host(), Some("localhost"));
    }
}
