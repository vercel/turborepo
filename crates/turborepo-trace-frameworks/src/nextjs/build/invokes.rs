//! Routes that a route invokes at runtime, and the effective fingerprint.
//!
//! A page's own output does not change when a route handler it fetches changes,
//! yet the page behaves differently. The *invoked routes* relation makes that
//! dependency explicit: per route, the routes its code can request while it
//! runs (a client `fetch` to a handler, a server component fetching the app's
//! own handler, a handler calling a handler, a data hook, a redirect). A
//! route's **effective** fingerprint is its own fingerprint combined
//! with the own fingerprints of everything it invokes, transitively.
//!
//! This module defines the relation's shape ([`InvokeRelation`]) and combines
//! it with a [`Fingerprint`] ([`apply`]). Producing the relation from source is
//! not done here: the static URL-site discovery ([`crate::nextjs::fetch`]) and
//! the recorder both feed it.
//!
//! What counts for the effective fingerprint, by [`InvokeKind`]:
//!
//! | Kind | Combined? | Why |
//! |---|---|---|
//! | `fetch`, `swr`, `query`, `trpc` | yes | the response is part of what the route shows or does |
//! | `redirect` | yes | visiting the route lands on the target |
//! | `action` | yes, and usually a no-op | a server action is bundled into the entries that use it, so its code is already in their own fingerprint |
//! | `navigation` | no | where a route can lead (`router.push`, `<Link>`) is not how it behaves; a test that follows it visits the target itself |
//!
//! Targets: a route of this build is combined; an external target (another
//! backend) is listed and ignored, since it is not part of this build; an
//! unknown target (a URL that could not be evaluated) stands for every handler
//! route and is reported.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::{
    fingerprint::{Fingerprint, hash_hex},
    manifest::{BuildRouteKind, BuildRouter, NextBuild},
};

/// How a route invokes another.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvokeKind {
    /// `fetch` and wrappers around it.
    Fetch,
    /// An SWR key.
    Swr,
    /// A React Query / TanStack Query function.
    Query,
    /// A typed remote procedure (tRPC).
    #[serde(alias = "rpc")]
    Trpc,
    /// A server action.
    Action,
    /// `redirect()`, `permanentRedirect()`, a redirecting response.
    Redirect,
    /// `router.push` / `router.replace`, `<Link href>`, `<Form
    /// action="/path">`.
    #[serde(alias = "navigate", alias = "link")]
    Navigation,
}

impl InvokeKind {
    /// Whether an invocation of this kind is part of the invoking route's
    /// behavior.
    pub fn combines(self) -> bool {
        self != InvokeKind::Navigation
    }
}

/// What is invoked.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(from = "TargetRepr", into = "TargetRepr")]
pub enum InvokeTarget {
    /// A route of this app, by source key ([`source_key`]) or by pathname.
    Route(String),
    /// A server action (`file#export`).
    Action(String),
    /// A remote procedure and the file that defines it. With the route that
    /// serves it (`serving`, the tRPC HTTP entry) it invokes that route;
    /// without, it stands for every handler route, like an unknown target.
    Procedure {
        /// The procedure path (`post.byId`).
        path: String,
        /// The router file.
        file: String,
        /// The route that serves the procedure, by source key or pathname.
        serving: Option<String>,
    },
    /// Something outside this build (an origin, a backend name).
    External(String),
    /// Not evaluable; the reason.
    Unknown(String),
}

/// The JSON shape of a target: an object with one of `route`, `action`,
/// `procedure` (+ `file`), `external` (a string, or `{origin, pattern}`),
/// `unknown`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct TargetRepr {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    route: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    procedure: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    serving: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    external: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    unknown: Option<String>,
}

impl From<TargetRepr> for InvokeTarget {
    fn from(r: TargetRepr) -> InvokeTarget {
        if let Some(x) = r.route {
            InvokeTarget::Route(x)
        } else if let Some(x) = r.action {
            InvokeTarget::Action(x)
        } else if let Some(path) = r.procedure {
            InvokeTarget::Procedure {
                path,
                file: r.file.unwrap_or_default(),
                serving: r.serving,
            }
        } else if let Some(x) = r.external {
            InvokeTarget::External(match x {
                serde_json::Value::String(s) => s,
                other => {
                    let part = |k: &str| other.get(k).and_then(|v| v.as_str()).unwrap_or("");
                    format!("{}{}", part("origin"), part("pattern"))
                }
            })
        } else {
            InvokeTarget::Unknown(r.unknown.unwrap_or_default())
        }
    }
}

impl From<InvokeTarget> for TargetRepr {
    fn from(t: InvokeTarget) -> TargetRepr {
        let mut r = TargetRepr::default();
        match t {
            InvokeTarget::Route(x) => r.route = Some(x),
            InvokeTarget::Action(x) => r.action = Some(x),
            InvokeTarget::Procedure {
                path,
                file,
                serving,
            } => {
                r.procedure = Some(path);
                r.file = Some(file);
                r.serving = serving;
            }
            InvokeTarget::External(x) => r.external = Some(serde_json::Value::String(x)),
            InvokeTarget::Unknown(x) => r.unknown = Some(x),
        }
        r
    }
}

/// How an invocation was found.
#[derive(
    Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Found {
    /// A call site in source.
    #[default]
    Static,
    /// A request seen by the recorder.
    Recorded,
    /// The build's manifests (server actions).
    Manifest,
}

/// One invocation.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Invocation {
    /// Kind.
    pub kind: InvokeKind,
    /// Target.
    pub target: InvokeTarget,
    /// How it was found.
    #[serde(default)]
    pub found: Found,
    /// The file of the call site (repo-relative), if static.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub file: String,
    /// Its line (1-based; 0 if unknown).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub line: u32,
    /// The HTTP method, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// The wrapper chain or API through which the URL reached a request
    /// (`useSWR`, `getItem → request → fetch`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub via: Vec<String>,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// Invocations per route. Keys are source keys ([`source_key`]:
/// `app/report/page`, `pages/legacy/[id]`) or pathnames (`/report`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvokeRelation {
    /// Format version (1).
    pub version: u32,
    /// The relation.
    pub routes: BTreeMap<String, Vec<Invocation>>,
}

impl InvokeRelation {
    /// An empty relation.
    pub fn new() -> InvokeRelation {
        InvokeRelation {
            version: 1,
            routes: BTreeMap::new(),
        }
    }

    /// Adds every invocation of `other`.
    pub fn merge(&mut self, other: InvokeRelation) {
        for (k, v) in other.routes {
            self.routes.entry(k).or_default().extend(v);
        }
    }
}

/// The server actions each entry bundles, from the build's server reference
/// manifest.
pub fn manifest_relation(build: &NextBuild) -> InvokeRelation {
    let mut rel = InvokeRelation::new();
    for a in &build.actions {
        for w in &a.workers {
            rel.routes.entry(w.clone()).or_default().push(Invocation {
                kind: InvokeKind::Action,
                target: InvokeTarget::Action(a.stable_id()),
                found: Found::Manifest,
                file: a.filename.clone(),
                line: 0,
                method: None,
                via: Vec::new(),
            });
        }
    }
    rel
}

/// The key of a route file in an [`InvokeRelation`]: its path relative to the
/// app root without the extension and without a leading `src/`
/// (`src/app/a/page.tsx` → `app/a/page`).
pub fn source_key(route_file: &str) -> String {
    let f = route_file.strip_prefix("src/").unwrap_or(route_file);
    match f.rfind('.') {
        Some(i) if !f[i..].contains('/') => f[..i].to_string(),
        _ => f.to_string(),
    }
}

/// The build's manifest key for a source key: `app/a/page` → (`App`,
/// `/a/page`); `pages/legacy/index` → (`Pages`, `/legacy`); `pages/index` →
/// (`Pages`, `/`).
pub fn manifest_key(source_key: &str) -> Option<(BuildRouter, String)> {
    if let Some(rest) = source_key.strip_prefix("app/") {
        return Some((BuildRouter::App, format!("/{rest}")));
    }
    let rest = source_key.strip_prefix("pages/")?;
    let rest = rest
        .strip_suffix("/index")
        .or_else(|| (rest == "index").then_some(""))
        .unwrap_or(rest);
    Some((BuildRouter::Pages, format!("/{rest}")))
}

/// What [`apply`] could not place.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyReport {
    /// Relation keys that are not routes of the build.
    pub unknown_routes: Vec<String>,
    /// Route targets that are not routes of the build (invoking pathname,
    /// target).
    pub unknown_targets: Vec<(String, String)>,
    /// Routes with an unevaluable invocation (they depend on every handler).
    pub open_routes: Vec<String>,
}

/// Resolves a relation key or route target to a pathname of `fp`.
fn resolve(fp: &Fingerprint, name: &str) -> Option<String> {
    if name.starts_with('/') {
        return fp.routes.contains_key(name).then(|| name.to_string());
    }
    let (router, key) = manifest_key(name)?;
    fp.routes
        .iter()
        .find(|(_, r)| r.router == router && r.key == key)
        .map(|(p, _)| p.clone())
}

/// Stores `relation` in `fp` (each route's `invokes`) and computes every
/// route's effective fingerprint: its own hash plus the pathnames and own
/// hashes of the routes it reaches through combining invocations. Cycles are
/// fine (reachability, not recursion).
pub fn apply(fp: &mut Fingerprint, relation: &InvokeRelation) -> ApplyReport {
    let mut report = ApplyReport::default();
    for r in fp.routes.values_mut() {
        r.invokes.clear();
    }
    let handlers: Vec<String> = fp
        .routes
        .iter()
        .filter(|(_, r)| r.kind == BuildRouteKind::Handler)
        .map(|(p, _)| p.clone())
        .collect();
    // Direct edges.
    let mut edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (key, invs) in &relation.routes {
        let Some(from) = resolve(fp, key) else {
            report.unknown_routes.push(key.clone());
            continue;
        };
        let mut list: Vec<Invocation> = Vec::new();
        let out = edges.entry(from.clone()).or_default();
        for inv in invs {
            let mut inv = inv.clone();
            match &inv.target {
                InvokeTarget::Route(t) => match resolve(fp, t) {
                    Some(to) => {
                        if inv.kind.combines() && to != from {
                            out.insert(to.clone());
                        }
                        inv.target = InvokeTarget::Route(to);
                    }
                    None => report.unknown_targets.push((from.clone(), t.clone())),
                },
                InvokeTarget::Procedure {
                    serving: Some(t), ..
                } if resolve(fp, t).is_some() => {
                    if let Some(to) = resolve(fp, t)
                        && inv.kind.combines()
                        && to != from
                    {
                        out.insert(to);
                    }
                }
                InvokeTarget::Unknown(_) | InvokeTarget::Procedure { .. }
                    if inv.kind.combines() =>
                {
                    out.extend(handlers.iter().filter(|h| **h != from).cloned());
                    if !report.open_routes.contains(&from) {
                        report.open_routes.push(from.clone());
                    }
                }
                _ => {}
            }
            list.push(inv);
        }
        list.sort();
        list.dedup();
        if let Some(r) = fp.routes.get_mut(&from) {
            r.invokes = list;
        }
    }
    // Effective = own + everything reachable.
    let own: BTreeMap<String, String> = fp
        .routes
        .iter()
        .map(|(p, r)| (p.clone(), r.own.clone()))
        .collect();
    for (path, r) in &mut fp.routes {
        let mut seen: BTreeSet<&String> = BTreeSet::new();
        let mut stack: Vec<&String> = edges.get(path).into_iter().flatten().collect();
        while let Some(t) = stack.pop() {
            if t != path && seen.insert(t) {
                stack.extend(edges.get(t).into_iter().flatten());
            }
        }
        r.effective = if seen.is_empty() {
            r.own.clone()
        } else {
            let mut buf = r.own.clone();
            for t in seen {
                buf.push('\n');
                buf.push_str(t);
                buf.push(' ');
                buf.push_str(&own[t]);
            }
            hash_hex(buf.as_bytes())
        };
    }
    report.unknown_routes.sort();
    report.open_routes.sort();
    report
}

/// The routes `path` reaches through combining invocations (transitively), from
/// the invocations stored in `fp`.
pub fn reaches(fp: &Fingerprint, path: &str) -> BTreeSet<String> {
    let handlers: Vec<&String> = fp
        .routes
        .iter()
        .filter(|(_, r)| r.kind == BuildRouteKind::Handler)
        .map(|(p, _)| p)
        .collect();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut stack: Vec<String> = vec![path.to_string()];
    while let Some(p) = stack.pop() {
        let Some(r) = fp.routes.get(&p) else { continue };
        for inv in r.invokes.iter().filter(|i| i.kind.combines()) {
            let targets: Vec<String> = match &inv.target {
                InvokeTarget::Route(t) => vec![t.clone()],
                InvokeTarget::Procedure {
                    serving: Some(t), ..
                } if resolve(fp, t).is_some() => resolve(fp, t).into_iter().collect(),
                InvokeTarget::Unknown(_) | InvokeTarget::Procedure { .. } => {
                    handlers.iter().map(|h| (*h).clone()).collect()
                }
                _ => Vec::new(),
            };
            for t in targets {
                if t != path && fp.routes.contains_key(&t) && seen.insert(t.clone()) {
                    stack.push(t);
                }
            }
        }
    }
    seen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys() {
        assert_eq!(source_key("app/report/page.tsx"), "app/report/page");
        assert_eq!(source_key("src/app/(m)/a.b/route.ts"), "app/(m)/a.b/route");
        assert_eq!(source_key("pages/legacy/[id].tsx"), "pages/legacy/[id]");
        assert_eq!(
            manifest_key("app/(m)/pricing/page"),
            Some((BuildRouter::App, "/(m)/pricing/page".to_string()))
        );
        assert_eq!(
            manifest_key("pages/legacy/index"),
            Some((BuildRouter::Pages, "/legacy".to_string()))
        );
        assert_eq!(
            manifest_key("pages/index"),
            Some((BuildRouter::Pages, "/".to_string()))
        );
        assert_eq!(
            manifest_key("pages/api/old"),
            Some((BuildRouter::Pages, "/api/old".to_string()))
        );
        assert_eq!(manifest_key("lib/x"), None);
    }

    #[test]
    fn kinds_that_combine() {
        for k in [
            InvokeKind::Fetch,
            InvokeKind::Swr,
            InvokeKind::Query,
            InvokeKind::Trpc,
            InvokeKind::Action,
            InvokeKind::Redirect,
        ] {
            assert!(k.combines());
        }
        assert!(!InvokeKind::Navigation.combines());
    }

    #[test]
    fn relation_json_shape() {
        // The shape agreed with the static extraction (extra fields are ignored).
        let json = r#"{"version":1,"stats":{"sites":3},"sites":{},"routes":{"app/report/page":[
            {"kind":"fetch","target":{"route":"app/api/report-data/route"},"file":"app/report/page.tsx","line":4,"method":"GET","via":["fetch"]},
            {"kind":"swr","target":{"external":{"origin":"https://api.example.com","pattern":"/v1/x"}},"file":"a.ts","line":1},
            {"kind":"trpc","target":{"procedure":"post.byId","file":"server/router.ts"}},
            {"kind":"navigation","target":{"route":"/about"}},
            {"kind":"query","target":{"unknown":"computed URL"}}]}}"#;
        let r: InvokeRelation = serde_json::from_str(json).unwrap();
        let v = &r.routes["app/report/page"];
        assert_eq!(
            v[0].target,
            InvokeTarget::Route("app/api/report-data/route".into())
        );
        assert_eq!(v[0].method.as_deref(), Some("GET"));
        assert_eq!(
            v[1].target,
            InvokeTarget::External("https://api.example.com/v1/x".into())
        );
        assert!(matches!(v[2].target, InvokeTarget::Procedure { .. }));
        assert_eq!(v[3].kind, InvokeKind::Navigation);
        assert_eq!(v[4].target, InvokeTarget::Unknown("computed URL".into()));
        let back: InvokeRelation =
            serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(back, r);
    }
}
