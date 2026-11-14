//! The invocation model: what a source file invokes at runtime, and what a
//! route invokes over its module closure.

use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::nextjs::urlpat::UrlPat;

/// The idiom of an invocation site.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvokeKind {
    /// `fetch`, a fetch wrapper, axios/ky/ofetch, `new Request`, `sendBeacon`,
    /// ...
    Fetch,
    /// An SWR hook (`useSWR`, `useSWRMutation`, `useSWRInfinite`, `mutate`,
    /// `preload`).
    Swr,
    /// A TanStack Query call (`useQuery`, `useMutation`, `prefetchQuery`, ...).
    Query,
    /// A tRPC procedure call.
    Trpc,
    /// A server action.
    Action,
    /// A server redirect or rewrite (`redirect()`, `NextResponse.redirect`).
    Redirect,
    /// A client navigation (`router.push`, `<Link href>`, `<Form
    /// action="/x">`): a visit only if a user or test performs it.
    Navigation,
}

impl InvokeKind {
    /// Whether the kind is an invocation the invoking route's behavior depends
    /// on. Plain navigations are listed but are not dependencies.
    pub fn is_dependency(self) -> bool {
        self != InvokeKind::Navigation
    }

    /// The JSON name.
    pub fn as_str(self) -> &'static str {
        match self {
            InvokeKind::Fetch => "fetch",
            InvokeKind::Swr => "swr",
            InvokeKind::Query => "query",
            InvokeKind::Trpc => "trpc",
            InvokeKind::Action => "action",
            InvokeKind::Redirect => "redirect",
            InvokeKind::Navigation => "navigation",
        }
    }
}

/// What a site targets, before mapping to the route table.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SiteTarget {
    /// A URL pattern.
    Url(UrlPat),
    /// A tRPC procedure path (`user.byId`).
    Procedure(String),
    /// A server action: the defining file (absolute) and its export name
    /// (`<inline:name>` for a function-level `'use server'`).
    Action {
        /// The file that defines the action.
        file: PathBuf,
        /// The export name.
        export: String,
    },
    /// The target could not be evaluated.
    Unknown(String),
}

/// One invocation site in a source file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Site {
    /// The file (absolute).
    pub file: PathBuf,
    /// 1-based line.
    pub line: u32,
    /// Idiom.
    pub kind: InvokeKind,
    /// The API at the site (`fetch`, `swr:default`, a wrapper's name).
    pub api: String,
    /// HTTP method, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// Targets (alternatives).
    pub targets: Vec<SiteTarget>,
    /// The wrapper chain from the site's callee down to the primitive API,
    /// outermost first (`["fetchTeam", "apiFetch", "fetch"]`). A direct
    /// call has one element.
    pub via: Vec<String>,
}

impl Site {
    /// Whether some alternative is unknown.
    pub fn has_unknown(&self) -> bool {
        self.targets
            .iter()
            .any(|t| matches!(t, SiteTarget::Unknown(_)))
    }
}

/// A string in a source file that looks like a URL path: a literal or template
/// that starts with `/`. The bound for requests whose URL cannot be evaluated:
/// such a request can only reach a route that some URL-like string of the
/// invoking route's module closure matches.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct UrlString {
    /// The file (absolute).
    pub file: PathBuf,
    /// 1-based line.
    pub line: u32,
    /// The pattern (a string ending in `/` is a prefix).
    pub pattern: UrlPat,
}

/// Where an invocation leads, after mapping through the route table.
///
/// JSON: an object with one of `route`, `action`, `procedure` (+ `file`,
/// `serving`), `external` (`{origin, pattern}`), `unknown`: the shape
/// [`crate::nextjs::build::invokes::InvokeRelation`] parses.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(from = "TargetRepr", into = "TargetRepr")]
pub enum Target {
    /// A route of this app: its key (the route file relative to the app root,
    /// without extension: `app/api/hello/route`, `pages/legacy/[id]`).
    Route(String),
    /// A server action: `<file relative to the graph root>#<export>`.
    Action(String),
    /// A tRPC procedure.
    Procedure {
        /// The procedure path (`user.byId`).
        path: String,
        /// The file that defines it (relative to the graph root); `None` if the
        /// router tree does not have it.
        file: Option<String>,
        /// The route that serves it over HTTP (the tRPC entry), if the app has
        /// one.
        serving: Option<String>,
    },
    /// Not a route of this app: another origin, another microfrontend, or a
    /// same-origin path no route serves (a backend behind the same host).
    External {
        /// `https://host`, `<origin>` (unknown), `mfe:<app>`, or `` for same-origin.
        origin: String,
        /// The endpoint pattern (`/v1/teams/*`).
        pattern: String,
    },
    /// Unknown: may be any handler.
    Unknown(String),
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct ExternalRepr {
    #[serde(default)]
    origin: String,
    #[serde(default)]
    pattern: String,
}

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
    external: Option<ExternalRepr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    unknown: Option<String>,
}

impl From<TargetRepr> for Target {
    fn from(r: TargetRepr) -> Target {
        if let Some(x) = r.route {
            Target::Route(x)
        } else if let Some(x) = r.action {
            Target::Action(x)
        } else if let Some(path) = r.procedure {
            Target::Procedure {
                path,
                file: r.file,
                serving: r.serving,
            }
        } else if let Some(x) = r.external {
            Target::External {
                origin: x.origin,
                pattern: x.pattern,
            }
        } else {
            Target::Unknown(r.unknown.unwrap_or_default())
        }
    }
}

impl From<Target> for TargetRepr {
    fn from(t: Target) -> TargetRepr {
        let mut r = TargetRepr::default();
        match t {
            Target::Route(x) => r.route = Some(x),
            Target::Action(x) => r.action = Some(x),
            Target::Procedure {
                path,
                file,
                serving,
            } => {
                r.procedure = Some(path);
                r.file = file;
                r.serving = serving;
            }
            Target::External { origin, pattern } => {
                r.external = Some(ExternalRepr { origin, pattern })
            }
            Target::Unknown(x) => r.unknown = Some(x),
        }
        r
    }
}

/// One invocation of a route: a target with its first witness site.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invocation {
    /// Idiom.
    pub kind: InvokeKind,
    /// Target.
    pub target: Target,
    /// HTTP method, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// The URL pattern at the site, for URL targets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The witness file (relative to the graph root); empty in the file table
    /// of the shared form, where the entry's file is the witness.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub file: String,
    /// Its 1-based line.
    pub line: u32,
    /// The wrapper chain at the witness.
    pub via: Vec<String>,
    /// How many sites in the route's closure have this kind and target.
    pub sites: u32,
    /// The target of a link or redirect only when the middleware puts segments
    /// in front of the path that Next's precedence would not pick (a fixed
    /// segment name, more than the route's leading dynamic segments): one
    /// of the routes the prefixed path can hit. Followed where nothing is
    /// assumed about the middleware (`--follow links`); not a target for
    /// the bounded rules and the build fingerprints. Not listed when the
    /// middleware itself redirects or rewrites to a URL that is not known: a
    /// request can then end on any page.
    #[serde(default, skip_serializing_if = "is_false")]
    pub wide: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// A tRPC procedure of the app's router tree.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Procedure {
    /// The path (`user.byId`).
    pub path: String,
    /// `query`, `mutation` or `subscription`.
    pub kind: String,
    /// The defining file (relative to the graph root).
    pub file: String,
    /// 1-based line.
    pub line: u32,
}

/// Counts over the sites of an app.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvokeStats {
    /// Files scanned.
    pub files: usize,
    /// Sites per kind.
    pub sites: BTreeMap<String, usize>,
    /// Dependency sites (every kind but navigation).
    pub dependency_sites: usize,
    /// Dependency sites with at least one route target.
    pub to_route: usize,
    /// Dependency sites with at least one handler route target.
    pub to_handler: usize,
    /// Dependency sites with an action target.
    pub to_action: usize,
    /// Dependency sites with a procedure target.
    pub to_procedure: usize,
    /// Dependency sites with an external target.
    pub external: usize,
    /// Dependency sites with an unknown alternative.
    pub unknown: usize,
    /// Unknown sites by reason class.
    pub unknown_classes: BTreeMap<String, usize>,
    /// Unknown sites by where the unknown URL comes from (`code`, `call`,
    /// `imported`, `param`, `env`, `response`, `input`, `storage`,
    /// `request`).
    #[serde(default)]
    pub unknown_sources: BTreeMap<String, usize>,
    /// External targets by origin.
    pub external_origins: BTreeMap<String, usize>,
    /// Derived wrappers (functions that forward a parameter to a URL API).
    pub wrappers: usize,
    /// Routes (pages and handlers).
    pub routes: usize,
    /// Routes with at least one unknown invocation in their closure.
    pub routes_with_unknown: usize,
    /// Pages.
    pub pages: usize,
    /// Pages with at least one unknown invocation in their closure.
    pub pages_with_unknown: usize,
    /// Distribution of the number of invoked routes per page: min, p25, median,
    /// p75, p90, max. With the bound, a page's unknown sites count as the
    /// routes of its bound; without, they are not counted here (they stand
    /// for every handler).
    pub page_invoked_routes: [usize; 6],
    /// Handler routes of the app.
    #[serde(default)]
    pub handlers: usize,
    /// Whether unknown targets were bounded by the URL-like strings of the
    /// closure.
    #[serde(default)]
    pub bounded: bool,
    /// Pages with an unknown request target that the bound does not cover: the
    /// URL is supplied by the request, storage or an input, so it stands
    /// for every handler.
    #[serde(default)]
    pub pages_unbounded: usize,
    /// Dependency sites with such a target.
    #[serde(default)]
    pub unbounded_sites: usize,
    /// The dependency sites with the most route targets (open patterns):
    /// `file:line`, the URL patterns, and the number of routes. The first
    /// places to look when pages invoke many routes.
    #[serde(default)]
    pub widest_sites: Vec<(String, String, usize)>,
    /// For pages with an unknown invocation: the distribution (min, p25,
    /// median, p75, p90, max) of the number of handler routes the URL-like
    /// strings of their closure match and no site of theirs already
    /// targets: what the bound adds instead of every handler.
    #[serde(default)]
    pub bound_added: [usize; 6],
    /// The same distribution for the total: site targets plus the bound.
    #[serde(default)]
    pub bound_total: [usize; 6],
}

/// The relation: per route, what it invokes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvokeRelation {
    /// Format version.
    pub version: u32,
    /// The app root, relative to the graph root.
    pub app: String,
    /// Route key → invocations over the route's module closure (deduplicated by
    /// kind and target, sorted).
    pub routes: BTreeMap<String, Vec<Invocation>>,
    /// Invocations of the middleware (it runs before every matched route).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub middleware: Vec<Invocation>,
    /// The app's tRPC procedures.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub procedures: Vec<Procedure>,
    /// Route keys of the tRPC HTTP entries (`app/api/trpc/[trpc]/route`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trpc_handlers: Vec<String>,
    /// Counts.
    pub stats: InvokeStats,
}
