//! Which Playwright specs a rebuilt Next.js app affects, from recorded visits
//! and build fingerprints.
//!
//! Validation tooling: this is the oracle's answer, which the selection from
//! source is checked against. It is not a way to select.
//!
//! A spec's recordings say which routes it reached on the build it ran against
//! ([`super::recorded`]). The spec must run again on a new build if what it
//! reached is not the same there. [`decide`] compares the two fingerprints for
//! one spec:
//!
//! | Difference | The spec runs if |
//! |---|---|
//! | a route's own output | it reached the route |
//! | the output of a route that a reached route calls where a recorder cannot see it | it reached the calling route |
//! | a removed route | it reached the route |
//! | an added route, or a changed route regex | a path it requested matches the new regex |
//! | an added or removed redirect, rewrite or header rule | a path it requested matches the rule's regex (conditions are not evaluated) |
//! | a `public/` file | it requested the file's path |
//! | the middleware or its matchers | a path it requested matches a matcher of either build |
//! | instrumentation, the evaluated config, the image config, `basePath`, i18n, the order of routes or rules | always |
//! | a request that could not be mapped to a route | always |
//!
//! A recorder sees what the browser and the test send. A redirect the browser
//! follows, a server action it posts and a navigation are requests of their
//! own, so the routes they lead to are in the recording when they happened.
//! What it cannot see is a request the server makes to the app itself: a
//! reached route counts as affected through the routes its code requests
//! (`fetch`, SWR, React Query, tRPC: any of them can run while the server
//! renders), transitively, and through nothing else.
//!
//! Every reason is a [`Why`], whose `Display` is the sentence `--explain`
//! prints.

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use serde::Serialize;
use serde_json::Value;

use super::recorded::{RequestKind, RouteMapper, Visits};
use crate::{
    nextjs::build::{
        diff::FingerprintDiff,
        fingerprint::Fingerprint,
        invokes::{InvokeKind, InvokeTarget},
        manifest::BuildRouteKind,
    },
    pattern::JsRegex,
};

/// Why a spec must run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Why {
    /// The spec has no recording, and its source does not say all it visits (or
    /// says it visits something that changed): the detail, if there is one.
    NoRecording {
        /// What the source says, when that is why the spec runs.
        detail: Option<String>,
    },
    /// Every recorded run of a test against the newest build failed.
    RecordedRunFailed {
        /// The test.
        test: String,
        /// Its outcome.
        status: String,
    },
    /// The recording does not say which build it ran against.
    NoBuildId {
        /// The test.
        test: String,
    },
    /// No fingerprint was given for the build a recording ran against.
    UnknownBuild {
        /// The build id.
        build: String,
    },
    /// No fingerprint of the build under test was given for the app a recording
    /// ran against.
    NoBuildUnderTest {
        /// The app directory.
        app: String,
    },
    /// The recording was made at a commit that is not in this clone, so the
    /// files that changed between it and the base revision are not known.
    UnknownCommit {
        /// The commit.
        commit: String,
    },
    /// The spec file is not the one that was recorded.
    SpecChanged,
    /// A recording could not be read.
    BadRecording {
        /// The reason.
        detail: String,
    },
    /// A reached route's output differs.
    RouteChanged {
        /// The route.
        route: String,
        /// The components that differ (`server`, `client`, `css`, `prerender`,
        /// `external`).
        components: Vec<String>,
        /// How the spec reached it.
        how: Reach,
    },
    /// A reached route invokes a route whose output differs.
    RouteInvokes {
        /// The reached route.
        route: String,
        /// The changed, added or removed routes it invokes.
        via: Vec<String>,
        /// How the spec reached it.
        how: Reach,
    },
    /// A reached route is gone.
    RouteRemoved {
        /// The route.
        route: String,
        /// How the spec reached it.
        how: Reach,
    },
    /// A requested path matches a route that was added or whose pattern
    /// changed.
    RouteAdded {
        /// The route.
        route: String,
        /// The requested path.
        path: String,
    },
    /// A requested path matches a routing rule that was added or removed.
    Rule {
        /// `added` or `removed`.
        change: &'static str,
        /// `redirect`, `rewrite` or `header`.
        rule: String,
        /// The rule's source pattern.
        source: String,
        /// The requested path.
        path: String,
    },
    /// A requested `public/` file was added, removed or modified.
    Public {
        /// The file, relative to `public/`.
        file: String,
        /// `added`, `removed` or `changed`.
        change: &'static str,
        /// The requested path.
        path: String,
    },
    /// The middleware (or its matchers) differ and a requested path passes
    /// through it.
    Middleware {
        /// The requested path, or `None` when the middleware runs on every
        /// path.
        path: Option<String>,
    },
    /// An input of every route differs.
    Global {
        /// What differs, in words.
        what: String,
    },
    /// A recorded request could not be mapped to a route.
    Unmapped {
        /// The requested path.
        path: String,
        /// The reason.
        detail: String,
    },
}

/// How a spec reached a route.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Reach {
    /// The request kind.
    pub kind: RequestKind,
    /// The method.
    pub method: String,
    /// The requested path.
    pub path: String,
}

impl Reach {
    /// `visits`, `requests`, `calls a server action on`.
    fn verb(&self) -> String {
        match self.kind {
            RequestKind::Document | RequestKind::Nav | RequestKind::Rsc | RequestKind::Data => {
                "visits".to_string()
            }
            RequestKind::Action => "calls a server action on".to_string(),
            RequestKind::Image => "loads an image from".to_string(),
            _ if self.method.is_empty() || self.method == "GET" => "requests".to_string(),
            _ => format!("sends {} to", self.method),
        }
    }

    /// The route with the concrete path when they differ: `/blog/[slug] (as
    /// /blog/a)`.
    fn target(&self, route: &str) -> String {
        if matches!(route, "/_not-found" | "/404" | "/_error") && self.path != route {
            format!("{} (no route matches it: the not-found page)", self.path)
        } else if self.path == route {
            route.to_string()
        } else {
            format!("{route} (as {})", self.path)
        }
    }
}

/// The components of a changed route in a developer's words.
fn components(c: &[String]) -> String {
    let words: Vec<&str> = c
        .iter()
        .map(|c| match c.as_str() {
            "server" => "server code",
            "client" => "client chunks",
            "css" => "styles",
            "prerender" => "prerendered output",
            "external" => "traced files",
            other => other,
        })
        .collect();
    match words.len() {
        0 => "output".to_string(),
        1 => words[0].to_string(),
        n => format!("{} and {}", words[..n - 1].join(", "), words[n - 1]),
    }
}

impl fmt::Display for Why {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Why::NoRecording { detail: None } => write!(f, "no recording"),
            Why::NoRecording { detail: Some(d) } => write!(f, "no recording; {d}"),
            Why::RecordedRunFailed { test, status } => write!(
                f,
                "the recorded run of \"{test}\" {}: its requests are incomplete",
                match status.as_str() {
                    "timedOut" => "timed out".to_string(),
                    "" => "did not finish".to_string(),
                    s => format!("ended as {s}"),
                }
            ),
            Why::NoBuildId { test } => write!(
                f,
                "the recording of \"{test}\" does not say which build it ran against (run the \
                 tests next to .next)"
            ),
            Why::UnknownBuild { build } => write!(
                f,
                "recorded against build {build}, whose fingerprint was not given \
                 (--base-fingerprint, or a fingerprint file in the visits directory)"
            ),
            Why::NoBuildUnderTest { app } => write!(
                f,
                "recorded against the app in {app}, for which no --next-fingerprint was given"
            ),
            Why::UnknownCommit { commit } => write!(
                f,
                "recorded at commit {}, which is not in this clone: what changed between it and \
                 --since is not known",
                &commit[..commit.len().min(10)]
            ),
            Why::SpecChanged => write!(f, "the spec file changed since it was recorded"),
            Why::BadRecording { detail } => write!(f, "a recording could not be read: {detail}"),
            Why::RouteChanged {
                route,
                components: c,
                how,
            } => write!(
                f,
                "{} {}, whose {} changed",
                how.verb(),
                how.target(route),
                components(c)
            ),
            Why::RouteInvokes { route, via, how } => write!(
                f,
                "{} {}, which invokes {}, which changed",
                how.verb(),
                how.target(route),
                via.join(", ")
            ),
            Why::RouteRemoved { route, how } => {
                write!(f, "{} {}, which was removed", how.verb(), how.target(route))
            }
            Why::RouteAdded { route, path } => {
                write!(f, "requests {path}, which the new route {route} matches")
            }
            Why::Rule {
                change,
                rule,
                source,
                path,
            } => write!(
                f,
                "requests {path}, which the {change} {rule} rule {source} matches"
            ),
            Why::Public { file, change, path } => {
                write!(f, "requests {path} (public/{file}), which was {change}")
            }
            Why::Middleware { path: Some(p) } => {
                write!(
                    f,
                    "middleware changed, and it runs on {p}, which the spec requests"
                )
            }
            Why::Middleware { path: None } => write!(f, "middleware changed"),
            Why::Global { what } => write!(f, "{what} changed"),
            Why::Unmapped { path, detail } => {
                write!(
                    f,
                    "requests {path}, which could not be mapped to a route: {detail}"
                )
            }
        }
    }
}

/// The parts of a fingerprint diff that are tested against requested paths,
/// compiled once.
#[derive(Debug)]
pub struct Comparison<'a> {
    base: &'a Fingerprint,
    head: &'a Fingerprint,
    diff: &'a FingerprintDiff,
    /// Reasons that hold for every spec.
    global: Vec<Why>,
    /// Added or re-patterned routes: (route, regex).
    added_routes: Vec<(String, JsRegex)>,
    /// Added and removed rules: (change, rule kind, source, regex).
    rules: Vec<(&'static str, String, String, JsRegex)>,
    /// The middleware differs: the matchers of both builds (`None`: it runs
    /// everywhere).
    middleware: Option<Option<Vec<JsRegex>>>,
    /// Routes whose own output changed, and added and removed routes.
    moved: BTreeSet<String>,
    /// Per route: the moved routes it requests out of a recorder's sight.
    unseen: RefCell<BTreeMap<String, Vec<String>>>,
}

fn matchers(fp: &Fingerprint) -> Option<Vec<JsRegex>> {
    let list = fp.middleware_matchers.as_ref()?.as_array()?;
    let mut out = Vec::new();
    for m in list {
        let re = JsRegex::new(m.get("regexp")?.as_str()?, "");
        if !re.is_supported() {
            return None;
        }
        out.push(re);
    }
    Some(out)
}

/// The label of a rule phase.
fn rule_kind(phase: &str) -> String {
    phase.split(':').next().unwrap_or(phase).to_string()
}

impl<'a> Comparison<'a> {
    /// Prepares the comparison of `base` (the build the recordings ran against)
    /// with `head`.
    pub fn new(
        base: &'a Fingerprint,
        head: &'a Fingerprint,
        diff: &'a FingerprintDiff,
    ) -> Comparison<'a> {
        let mut global = Vec::new();
        let mut middleware = None;
        let case = if base.case_sensitive { "" } else { "i" };
        let mut rules = Vec::new();
        let mut added_routes = Vec::new();
        for g in &diff.global {
            match g.as_str() {
                "middleware" | "middleware_matchers" => {
                    if middleware.is_some() {
                        continue;
                    }
                    // A build without middleware has no matchers; one with middleware and
                    // unreadable matchers runs it everywhere.
                    let sides: Vec<Option<Vec<JsRegex>>> = [base, head]
                        .iter()
                        .filter(|fp| fp.global.contains_key("middleware"))
                        .map(|fp| matchers(fp))
                        .collect();
                    middleware = Some(if sides.iter().any(Option::is_none) {
                        None
                    } else {
                        Some(sides.into_iter().flatten().flatten().collect())
                    });
                }
                "routing" => {
                    if diff.rules.added.is_empty() && diff.rules.removed.is_empty() {
                        global.push(Why::Global {
                            what: "the routing configuration (basePath, i18n or the order of \
                                   rules)"
                                .to_string(),
                        });
                    }
                }
                "route_table" => {
                    if diff.route_table.added.is_empty() && diff.route_table.removed.is_empty() {
                        global.push(Why::Global {
                            what: "the order of the routes".to_string(),
                        });
                    }
                }
                "instrumentation" => global.push(Why::Global {
                    what: "instrumentation".to_string(),
                }),
                "config" => global.push(Why::Global {
                    what: "the Next.js config".to_string(),
                }),
                "images" => global.push(Why::Global {
                    what: "the image config".to_string(),
                }),
                other => global.push(Why::Global {
                    what: other.to_string(),
                }),
            }
        }
        for (change, list) in [
            ("added", &diff.rules.added),
            ("removed", &diff.rules.removed),
        ] {
            for r in list {
                let re = JsRegex::new(r.regex.clone(), case);
                if re.is_supported() {
                    rules.push((change, rule_kind(&r.phase), r.source.clone(), re));
                } else {
                    global.push(Why::Global {
                        what: format!("the {} rule {}", rule_kind(&r.phase), r.source),
                    });
                }
            }
        }
        for r in &diff.route_table.added {
            let re = JsRegex::new(r.regex.clone(), "");
            if re.is_supported() {
                added_routes.push((r.page.clone(), re));
            } else {
                global.push(Why::Global {
                    what: format!("the route {}", r.page),
                });
            }
        }
        if base.base_path != head.base_path || base.i18n != head.i18n {
            let what = "the routing configuration (basePath, i18n or the order of rules)";
            if !global
                .iter()
                .any(|w| matches!(w, Why::Global { what: x } if x == what))
            {
                global.push(Why::Global {
                    what: what.to_string(),
                });
            }
        }
        let moved = diff
            .changed
            .keys()
            .chain(&diff.added)
            .chain(&diff.removed)
            .cloned()
            .collect();
        Comparison {
            base,
            head,
            diff,
            global,
            added_routes,
            rules,
            middleware,
            moved,
            unseen: RefCell::new(BTreeMap::new()),
        }
    }

    /// The changed, added or removed routes that `route` requests,
    /// transitively, where a recorder cannot see it: through `fetch`, SWR,
    /// React Query and tRPC call sites of either build (a URL that could
    /// not be evaluated stands for every handler). Redirects,
    /// server actions and navigations are requests of the browser.
    fn unseen(&self, route: &str) -> Vec<String> {
        if self.moved.is_empty() {
            return Vec::new();
        }
        if let Some(hit) = self.unseen.borrow().get(route) {
            return hit.clone();
        }
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for fp in [self.base, self.head] {
            let mut stack = vec![route.to_string()];
            let mut done: BTreeSet<String> = BTreeSet::new();
            while let Some(p) = stack.pop() {
                let Some(r) = fp.routes.get(&p) else {
                    continue;
                };
                for inv in &r.invokes {
                    if !matches!(
                        inv.kind,
                        InvokeKind::Fetch | InvokeKind::Swr | InvokeKind::Query | InvokeKind::Trpc
                    ) {
                        continue;
                    }
                    let targets: Vec<&String> = match &inv.target {
                        InvokeTarget::Route(t) => vec![t],
                        InvokeTarget::Unknown(_) | InvokeTarget::Procedure { .. } => fp
                            .routes
                            .iter()
                            .filter(|(_, r)| r.kind == BuildRouteKind::Handler)
                            .map(|(p, _)| p)
                            .collect(),
                        InvokeTarget::Action(_) | InvokeTarget::External(_) => Vec::new(),
                    };
                    for t in targets {
                        if t != route && done.insert(t.clone()) {
                            seen.insert(t.clone());
                            stack.push(t.clone());
                        }
                    }
                }
            }
        }
        let hit: Vec<String> = seen
            .into_iter()
            .filter(|t| self.moved.contains(t))
            .collect();
        self.unseen
            .borrow_mut()
            .insert(route.to_string(), hit.clone());
        hit
    }

    /// Reasons that hold for every spec, whatever it visited.
    pub fn global(&self) -> &[Why] {
        &self.global
    }

    /// The diff.
    pub fn diff(&self) -> &FingerprintDiff {
        self.diff
    }

    /// Why a spec that reached `visits` on the base build must run on the head
    /// build. Empty: what it reached is the same.
    pub fn decide(&self, visits: &Visits, mapper: &RouteMapper<'_>) -> Vec<Why> {
        let mut out: Vec<Why> = self.global.clone();
        match &self.middleware {
            Some(None) => out.push(Why::Middleware { path: None }),
            Some(Some(res)) => {
                if let Some(p) = visits
                    .paths
                    .iter()
                    .find(|p| res.iter().any(|re| re.is_match(p)))
                {
                    out.push(Why::Middleware {
                        path: Some(p.clone()),
                    });
                }
            }
            None => {}
        }
        for (route, e) in &visits.routes {
            let how = Reach {
                kind: e.kind,
                method: e.method.clone(),
                path: e.path.clone(),
            };
            if let Some(c) = self.diff.changed.get(route) {
                out.push(Why::RouteChanged {
                    route: route.clone(),
                    components: c.clone(),
                    how,
                });
            } else if self.diff.removed.contains(route) || !self.head.routes.contains_key(route) {
                out.push(Why::RouteRemoved {
                    route: route.clone(),
                    how,
                });
            } else {
                let via = self.unseen(route);
                if !via.is_empty() {
                    out.push(Why::RouteInvokes {
                        route: route.clone(),
                        via,
                        how,
                    });
                }
            }
        }
        // One reason per rule and per added route is enough.
        let mut seen: BTreeSet<usize> = BTreeSet::new();
        for p in &visits.paths {
            for (i, (change, kind, source, re)) in self.rules.iter().enumerate() {
                if !seen.contains(&i) && re.is_match(p) {
                    seen.insert(i);
                    out.push(Why::Rule {
                        change,
                        rule: kind.clone(),
                        source: source.clone(),
                        path: p.clone(),
                    });
                }
            }
        }
        let mut seen: BTreeSet<usize> = BTreeSet::new();
        for p in &visits.paths {
            let Some(rp) = mapper.route_path(p) else {
                continue;
            };
            for (i, (route, re)) in self.added_routes.iter().enumerate() {
                // A route the spec already reached is covered by its own comparison.
                if !seen.contains(&i) && !visits.routes.contains_key(route) && re.is_match(&rp) {
                    seen.insert(i);
                    out.push(Why::RouteAdded {
                        route: route.clone(),
                        path: p.clone(),
                    });
                }
            }
            // A file added to `public/` answers before any route.
            let file = rp.trim_start_matches('/');
            if !file.is_empty() {
                let (b, h) = (self.base.public.get(file), self.head.public.get(file));
                if b != h {
                    let change = match (b, h) {
                        (None, _) => "added",
                        (_, None) => "removed",
                        _ => "changed",
                    };
                    if !out
                        .iter()
                        .any(|w| matches!(w, Why::Public { file: x, .. } if x == file))
                    {
                        out.push(Why::Public {
                            file: file.to_string(),
                            change,
                            path: p.clone(),
                        });
                    }
                }
            }
        }
        for (path, detail) in &visits.unmapped {
            if !out.iter().any(|w| matches!(w, Why::Unmapped { .. })) {
                out.push(Why::Unmapped {
                    path: path.clone(),
                    detail: detail.clone(),
                });
            }
        }
        out
    }
}

/// The middleware matchers as source patterns, for messages.
pub fn matcher_sources(fp: &Fingerprint) -> Vec<String> {
    fp.middleware_matchers
        .as_ref()
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|m| m.get("originalSource")?.as_str().map(str::to_string))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        nextjs::build::diff::{CssMode, diff},
        playwright::recorded::{Recording, Request, RouteMapper},
    };

    fn fp(build: &str, edit: impl FnOnce(&mut Value)) -> Fingerprint {
        let route = |own: &str| {
            serde_json::json!({
                "key": "k", "router": "app", "kind": "page",
                "server": {"hash": format!("s-{own}"), "files": 1},
                "client": {"hash": "c", "files": 1},
                "css": {"hash": "x", "files": 0}, "prerender": {"hash": "p", "files": 0},
                "external": "e", "own": own, "effective": own
            })
        };
        let mut v = serde_json::json!({
            "version": 3, "build_id": build, "bundler": "turbopack", "per_module": true,
            "routes": {
                "/": route("home"), "/about": route("about"), "/blog/[slug]": route("blog"),
                "/api/hello": route("hello"), "/_not-found": route("nf")
            },
            "global": {"middleware": "m1", "middleware_matchers": "mm", "routing": "r1", "route_table": "t1", "config": "c1", "images": "i1"},
            "rules": [],
            "route_regexes": [
                {"page": "/", "regex": "^/(?:/)?$"},
                {"page": "/_not-found", "regex": "^/_not\\-found(?:/)?$"},
                {"page": "/about", "regex": "^/about(?:/)?$"},
                {"page": "/api/hello", "regex": "^/api/hello(?:/)?$"},
                {"page": "/blog/[slug]", "regex": "^/blog/([^/]+?)(?:/)?$"}
            ],
            "middleware_matchers": [{"regexp": "^(?:\\/(_next\\/data\\/[^/]{1,}))?\\/dashboard(?:\\/.*)?[\\/#\\?]?$", "originalSource": "/dashboard/:path*"}],
            "public": {"logo.png": "h1"}
        });
        edit(&mut v);
        serde_json::from_value(v).unwrap()
    }

    fn visits(base: &Fingerprint, paths: &[(&str, RequestKind)]) -> Visits {
        let rec = Recording {
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
            requests: paths
                .iter()
                .map(|(p, k)| Request {
                    kind: *k,
                    method: "GET".into(),
                    path: (*p).into(),
                    status: Some(200),
                    host: None,
                    query: Vec::new(),
                    req: Default::default(),
                    res: Default::default(),
                    target: None,
                    from: None,
                    hook: false,
                    n: 1,
                })
                .collect(),
            file: Default::default(),
        };
        let m = RouteMapper::new(base);
        let mut v = Visits::default();
        v.add(&m, &rec);
        v
    }

    fn reasons(
        base: &Fingerprint,
        head: &Fingerprint,
        paths: &[(&str, RequestKind)],
    ) -> Vec<String> {
        let d = diff(base, head, CssMode::Utilities).unwrap();
        let c = Comparison::new(base, head, &d);
        let m = RouteMapper::new(base);
        c.decide(&visits(base, paths), &m)
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    const ABOUT: &[(&str, RequestKind)] = &[
        ("/about", RequestKind::Document),
        ("/_next/static/chunks/a.js", RequestKind::Asset),
        ("/logo.png", RequestKind::Fetch),
    ];

    #[test]
    fn unchanged_build_selects_nothing() {
        let base = fp("b1", |_| {});
        let head = fp("b2", |_| {});
        assert!(reasons(&base, &head, ABOUT).is_empty());
    }

    #[test]
    fn a_changed_route_selects_its_visitors_only() {
        let base = fp("b1", |_| {});
        let head = fp("b2", |v| {
            v["routes"]["/blog/[slug]"]["server"]["hash"] = "s-new".into();
            v["routes"]["/blog/[slug]"]["own"] = "new".into();
            v["routes"]["/blog/[slug]"]["effective"] = "new".into();
        });
        assert!(reasons(&base, &head, ABOUT).is_empty());
        assert_eq!(
            reasons(&base, &head, &[("/blog/first", RequestKind::Nav)]),
            ["visits /blog/[slug] (as /blog/first), whose server code changed"]
        );
    }

    #[test]
    fn removed_and_added_routes() {
        let base = fp("b1", |_| {});
        let head = fp("b2", |v| {
            v["routes"].as_object_mut().unwrap().remove("/about");
            v["route_regexes"].as_array_mut().unwrap().remove(2);
            v["global"]["route_table"] = "t2".into();
        });
        assert_eq!(
            reasons(&base, &head, ABOUT),
            ["visits /about, which was removed"]
        );
        assert!(reasons(&base, &head, &[("/", RequestKind::Document)]).is_empty());
        // A new static route takes a path a dynamic route served.
        let head = fp("b2", |v| {
            v["routes"]["/blog/new"] = v["routes"]["/about"].clone();
            v["route_regexes"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::json!({"page": "/blog/new", "regex": "^/blog/new(?:/)?$"}));
            v["global"]["route_table"] = "t2".into();
        });
        assert_eq!(
            reasons(&base, &head, &[("/blog/new", RequestKind::Document)]),
            ["requests /blog/new, which the new route /blog/new matches"]
        );
        assert!(reasons(&base, &head, &[("/blog/old", RequestKind::Document)]).is_empty());
    }

    #[test]
    fn rules_public_files_and_middleware() {
        let base = fp("b1", |_| {});
        let head = fp("b2", |v| {
            v["rules"] = serde_json::json!([{"phase": "redirect", "source": "/about", "regex": "^/about(?:/)?$", "destination": "/", "internal": false, "raw": {"source": "/about"}}]);
            v["global"]["routing"] = "r2".into();
        });
        assert_eq!(
            reasons(&base, &head, ABOUT),
            ["requests /about, which the added redirect rule /about matches"]
        );
        assert!(reasons(&base, &head, &[("/", RequestKind::Document)]).is_empty());

        let head = fp("b2", |v| v["public"]["logo.png"] = "h2".into());
        assert_eq!(
            reasons(&base, &head, ABOUT),
            ["requests /logo.png (public/logo.png), which was changed"]
        );
        assert!(reasons(&base, &head, &[("/", RequestKind::Document)]).is_empty());

        // The middleware runs on /dashboard only.
        let head = fp("b2", |v| v["global"]["middleware"] = "m2".into());
        assert!(reasons(&base, &head, ABOUT).is_empty());
        assert_eq!(
            reasons(&base, &head, &[("/dashboard/x", RequestKind::Document)]),
            [
                "middleware changed, and it runs on /dashboard/x, which the spec requests",
                // No route matches /dashboard/x in this table.
            ]
        );
        // Without readable matchers it runs everywhere.
        let head = fp("b2", |v| {
            v["global"]["middleware"] = "m2".into();
            v.as_object_mut().unwrap().remove("middleware_matchers");
        });
        assert_eq!(reasons(&base, &head, ABOUT), ["middleware changed"]);
    }

    #[test]
    fn invoked_routes_count_only_where_a_recorder_cannot_see() {
        // The home page redirects to /about and fetches /api/hello in its code.
        let invokes = |v: &mut Value| {
            v["routes"]["/"]["invokes"] = serde_json::json!([
                {"kind": "redirect", "target": {"route": "/about"}},
                {"kind": "fetch", "target": {"route": "/api/hello"}}
            ]);
        };
        let change = |route: &'static str| {
            move |v: &mut Value| {
                invokes(v);
                v["routes"][route]["server"]["hash"] = "s-new".into();
                v["routes"][route]["own"] = "new".into();
            }
        };
        let base = fp("b1", invokes);
        let home: &[(&str, RequestKind)] = &[("/", RequestKind::Document)];
        // The redirect target changed: had the spec been redirected, /about would be in
        // its recording.
        assert!(reasons(&base, &fp("b2", change("/about")), home).is_empty());
        // The handler the page's code fetches changed: the request may be the server's.
        assert_eq!(
            reasons(&base, &fp("b2", change("/api/hello")), home),
            ["visits /, which invokes /api/hello, which changed"]
        );
    }

    #[test]
    fn global_inputs_select_everything() {
        let base = fp("b1", |_| {});
        let head = fp("b2", |v| v["global"]["config"] = "c2".into());
        assert_eq!(reasons(&base, &head, ABOUT), ["the Next.js config changed"]);
        let head = fp("b2", |v| v["global"]["instrumentation"] = "x".into());
        assert_eq!(reasons(&base, &head, &[]), ["instrumentation changed"]);
    }
}
