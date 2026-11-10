//! `affected --tests playwright` without a build and without a
//! recording: which end-to-end specs a changed module reaches, from the source
//! alone.
//!
//! ```text
//! changed module → importing chain → route entry file → route → the spec's line that visits it
//! ```
//!
//! Two static relations meet at the routes of the Next.js apps under the root:
//!
//! **Module → routes.** Every file Next.js composes a route from (the page or
//! handler, the layouts, templates and boundaries above it, the pages and
//! defaults of parallel slots, metadata files, `pages/_app` and `_document`) is
//! an entry of the ordinary `affected` evaluation, next to the specs: a route
//! is *reached by the change* when one of its entry files is affected. Nothing
//! is traced a second time: imports, `import()`, `next/dynamic`, re-exports
//! through barrels, server actions, package versions from the lockfile, the
//! barrel and erasure rules and the uncertainty policy are those of every other
//! entry. The app's middleware, instrumentation, `next.config` and stylesheet
//! pipeline configs are entries too: a change that reaches one of them reaches
//! every route of the app. Route code runs in the app directory, so that is the
//! working directory of these entries.
//!
//! **Spec → routes.** The URLs a spec names
//! ([`turborepo_trace_frameworks::playwright::visits`]: `page.goto`, page
//! objects, request calls, link clicks, URL assertions, in the spec and
//! the helpers it imports) are matched against the route table of the app the
//! spec's Playwright config belongs to
//! ([`turborepo_trace_frameworks::nextjs::matcher`]: dynamic segments, route
//! groups, redirects and rewrites of the config, microfrontends). From
//! there the visit continues along the static invocation relation
//! ([`turborepo_trace_frameworks::nextjs::invoke`]), by [`Follow`]:
//!
//! | [`Follow`] | A spec visits |
//! |---|---|
//! | `Named` | the routes its URLs match |
//! | `Calls` | plus what those routes request at run time, transitively: `fetch`/SWR/React Query/tRPC targets, server actions, the bound of an unknown request URL |
//! | `Redirects` | plus where their code (and the middleware) redirects, when the target is one known URL |
//! | `Links` (the default) | plus every redirect target and the pages those pages can navigate to (`<Link>`, `router.push`, form actions), transitively |
//!
//! The target of a redirect or a link is requested like a URL a spec names, so
//! both are matched the same way, the prefix a middleware can put in front of a
//! path included (`Matcher::behind_prefix`): `redirect('/login')` under a
//! middleware that rewrites to `/<locale>/login` leads to `/[locale]/login`.
//! The routes Next's precedence picks behind leading dynamic segments are
//! targets under every rule; every other route the prefixed path can hit is a
//! target under `Links` only (`Invocation::wide`).
//!
//! A spec *may visit any route* of its apps when a navigation target cannot be
//! bounded: a URL that cannot be evaluated, no navigation at all, and under
//! `Links` a redirect or link whose target is unknown, a middleware that builds
//! its target, a config rule that cannot be read. Such a spec runs when the
//! change reaches any route of its apps.
//!
//! Only `Links` is free of assumptions about where a spec goes: the other
//! rules skip specs that reach a changed page by a redirect or a click without
//! naming its URL. They also take an unknown URL part that is a fixture of the
//! test callback, an environment variable or a call result as a name of data
//! (it matches dynamic segments only), and do not look for a URL that names a
//! static route behind a middleware prefix. On a large app `Links` leaves
//! nearly every spec at "may visit any route", so its answer is per app.
//!
//! One assumption holds under every rule: a build-time config (`next.config`
//! and what it loads, the Tailwind and PostCSS configs) counts as changed
//! through the modules it loads and the files it names, not through every file
//! below a directory it lists or computes a path into (`--strict` counts
//! those).
//!
//! [`Static::decide`] gives, per spec, the first reason through a route (or
//! none); `crate::e2e` adds the spec's own verdict, its Playwright setup,
//! screenshots and environment files.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::json;
use turborepo_trace_frameworks::{
    nextjs::{
        GlobalWhy, NextApp, detect,
        invoke::{self, InvokeKind, SharedRelation, Target},
        is_next_app,
        jseval::Files,
        matcher::{Ambiguity, Matcher, Outcome},
        routes::RouteKind,
        select::GraphImports,
        urlpat::{Origin, Part},
    },
    playwright::visits::{ExtractOptions, HelperCache, SpecVisits, extract_visits_cached},
};
use turborepo_trace_graph::{
    AffectedReport, Cause, Change, ChangeKind, EntryResult, GraphView, NodeId, RefFlags, RefKind,
};
use turborepo_trace_store::Graph;

use crate::e2e_cache;

/// How far a visit is followed from the routes a spec names.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Follow {
    /// Only the routes the spec's URLs match.
    Named,
    /// Plus the routes those routes request at run time.
    Calls,
    /// Plus where their code redirects to, when the target is one known URL.
    Redirects,
    /// Plus every redirect target and the pages they can navigate to.
    Links,
}

impl Follow {
    pub(crate) const ALL: [Follow; 4] = [
        Follow::Named,
        Follow::Calls,
        Follow::Redirects,
        Follow::Links,
    ];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Follow::Named => "named",
            Follow::Calls => "calls",
            Follow::Redirects => "redirects",
            Follow::Links => "links",
        }
    }

    /// The rule in force: `--follow`, else `Links` (the only one without an
    /// assumption about where a spec goes). `TURBO_TRACE_DEBUG_E2E_FOLLOW`
    /// overrides both, for measurements.
    pub(crate) fn rule(flag: Option<Follow>) -> Follow {
        match turborepo_trace_core::debug::var("E2E_FOLLOW").as_deref() {
            Ok("named") => Follow::Named,
            Ok("calls") => Follow::Calls,
            Ok("redirects") => Follow::Redirects,
            Ok("links") => Follow::Links,
            _ => flag.unwrap_or(Follow::Links),
        }
    }
}

/// An input of every route of an app.
#[derive(Debug)]
struct Global {
    /// Root-relative path.
    path: String,
    /// What it is, for the reason text.
    what: &'static str,
    /// Code: an entry of the evaluation. Otherwise the file itself has to be
    /// changed.
    code: bool,
    /// Runs when the app is built or started, not per request: `next.config`
    /// and its helpers, the stylesheet pipeline configs. A change counts
    /// when the config's code or a file it names changed; a path it
    /// computes or a directory it lists does not make every file below a
    /// change of the config (`--strict` counts it).
    build_time: bool,
}

/// One Next.js app.
#[derive(Debug)]
struct App {
    /// Root-relative directory (empty: the root).
    dir: String,
    next: NextApp,
    /// Per route of `next.table.routes`: its entry files (root-relative) on a
    /// hard navigation, and the extra ones on a soft navigation
    /// (intercepting routes).
    files: Vec<(Vec<String>, Vec<String>)>,
    globals: Vec<Global>,
    /// Route key ([`invoke::route_key`]) → index.
    by_key: BTreeMap<String, usize>,
    /// Route id → index.
    by_id: BTreeMap<String, usize>,
}

impl App {
    fn in_root(&self, f: &str) -> String {
        if self.dir.is_empty() {
            f.to_string()
        } else {
            format!("{}/{f}", self.dir)
        }
    }

    fn label(&self) -> &str {
        if self.dir.is_empty() { "." } else { &self.dir }
    }
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

fn global_what(why: GlobalWhy) -> Option<(&'static str, bool)> {
    // (text, an entry of the evaluation even if `code` is not set)
    Some(match why {
        GlobalWhy::NextConfig => ("the Next.js config", true),
        GlobalWhy::NextConfigHelper => ("a module the Next.js config loads", false),
        GlobalWhy::Middleware => ("the middleware, which runs before every route", true),
        GlobalWhy::Instrumentation => ("the instrumentation, which every route loads", true),
        GlobalWhy::TailwindConfig => ("the Tailwind config (the stylesheet of every route)", true),
        GlobalWhy::PostCss => ("the PostCSS config (the stylesheet of every route)", true),
        GlobalWhy::Microfrontends => ("the microfrontends config (which app serves a URL)", false),
        GlobalWhy::VercelConfig => ("the Vercel config (redirects, rewrites, headers)", false),
        // Entry files of the routes they wrap: in the routes' own closures.
        GlobalWhy::RootLayout | GlobalWhy::RootBoundary | GlobalWhy::PagesSpecial => return None,
        // Imported by a layout (a graph edge) or consulted by resolution (the manifest and
        // tsconfig are inputs of the entries that resolve through them).
        GlobalWhy::GlobalCss | GlobalWhy::PackageJson | GlobalWhy::Tsconfig => return None,
    })
}

/// Where an invocation of a route leads.
#[derive(Clone, Debug, Serialize, Deserialize)]
enum EdgeTo {
    /// A route of the same app.
    Route(usize),
    /// A URL that is not known: any page (the reason).
    AnyPage(String),
    /// A request whose URL comes from outside the code: any handler (the
    /// reason).
    AnyHandler(String),
}

/// One invocation of a route, reduced to what a visit follows.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Edge {
    to: EdgeTo,
    kind: InvokeKind,
    /// `file:line` of the site.
    at: String,
    /// The URL at the site is one known URL (no unknown part).
    exact: bool,
    /// The target only when the middleware puts segments in front of the path
    /// that Next's precedence would not pick (`Invocation::wide`): followed
    /// under `Links`.
    wide: bool,
}

/// The relation of one app. A route's edges are those of the file sets it
/// refers to (a route has at most two: what its layouts reach, shared with its
/// siblings, and what its own file reaches), so they are computed per set, on
/// first use.
struct AppRel {
    rel: SharedRelation,
    middleware: Vec<Edge>,
    sets: std::cell::RefCell<BTreeMap<u32, std::rc::Rc<Vec<Edge>>>>,
    /// Route index in the relation's `route_keys` → index in the app's table.
    by_rel_index: Vec<Option<usize>>,
    /// Segments the middleware's rewrites put in front of an open path.
    prefixes: Vec<invoke::Prefix>,
}

impl AppRel {
    fn new(
        app: &App,
        rel: SharedRelation,
        middleware: Vec<Edge>,
        prefixes: Vec<invoke::Prefix>,
    ) -> AppRel {
        let by_rel_index = rel
            .route_keys
            .iter()
            .map(|k| app.by_key.get(k).copied())
            .collect();
        AppRel {
            rel,
            middleware,
            sets: std::cell::RefCell::new(BTreeMap::new()),
            by_rel_index,
            prefixes,
        }
    }

    fn set_edges(&self, app: &App, si: u32) -> std::rc::Rc<Vec<Edge>> {
        if let Some(e) = self.sets.borrow().get(&si) {
            return e.clone();
        }
        let mut out: Vec<Edge> = Vec::new();
        let mut seen: BTreeSet<(u8, usize, bool, bool, bool, bool)> = BTreeSet::new();
        for &fi in &self.rel.sets[si as usize] {
            let f = &self.rel.files[fi as usize];
            for inv in &f.invokes {
                let nav = inv.kind == InvokeKind::Navigation;
                let at = || format!("{}:{}", f.file, inv.line);
                let to = match &inv.target {
                    Target::Route(k) => app.by_key.get(k).map(|&t| EdgeTo::Route(t)),
                    Target::Procedure {
                        serving: Some(k), ..
                    } => app.by_key.get(k).map(|&t| EdgeTo::Route(t)),
                    Target::Unknown(_) if nav || inv.kind == InvokeKind::Redirect => {
                        Some(EdgeTo::AnyPage(format!(
                            "{}: code of the page {} a URL that is not known",
                            at(),
                            if nav {
                                "can navigate to"
                            } else {
                                "redirects to"
                            }
                        )))
                    }
                    // Other unknown targets: the bound of the relation stands for them.
                    _ => None,
                };
                let Some(to) = to else { continue };
                let key = match &to {
                    EdgeTo::Route(t) => (0, *t, nav),
                    EdgeTo::AnyPage(_) => (1, 0, nav),
                    EdgeTo::AnyHandler(_) => (2, 0, nav),
                };
                let exact = inv.url.as_deref().is_some_and(|u| !u.contains('*'));
                // One edge per target and kind of following (a link, a redirect, a
                // request); one unknown of each kind is enough.
                let redirect = inv.kind == InvokeKind::Redirect;
                let key = (key.0, key.1, key.2, redirect, exact, inv.wide);
                if seen.insert(key) {
                    out.push(Edge {
                        to,
                        kind: inv.kind,
                        at: at(),
                        exact,
                        wide: inv.wide,
                    });
                }
            }
        }
        let out = std::rc::Rc::new(out);
        self.sets.borrow_mut().insert(si, out.clone());
        out
    }

    /// Calls `f` with every edge of route `ri`.
    fn for_edges(&self, app: &App, ri: usize, mut f: impl FnMut(&Edge)) {
        let route = &app.next.table.routes[ri];
        let Some(sets) = self.rel.routes.get(&invoke::route_key(route)) else {
            return;
        };
        if sets.open {
            f(&Edge {
                to: EdgeTo::AnyHandler(format!(
                    "{} makes a request whose URL comes from outside the code",
                    route.pattern
                )),
                kind: InvokeKind::Fetch,
                at: String::new(),
                exact: false,
                wide: false,
            });
        }
        for &si in &sets.sets {
            for e in self.set_edges(app, si).iter() {
                f(e);
            }
        }
        // The bound of the unknown request targets: handlers that URL-like strings of
        // the route's code match.
        for &k in &sets.bound {
            if let Some(Some(t)) = self.by_rel_index.get(k as usize) {
                f(&Edge {
                    to: EdgeTo::Route(*t),
                    kind: InvokeKind::Fetch,
                    at: "a URL-like string of its code".to_string(),
                    exact: false,
                    wide: false,
                });
            }
        }
    }
}

/// A spec's verdict for the summary: its path, and for a selected spec the kind
/// of its first reason (for the spec's own code and its Playwright setup with
/// `_named` when a file changed that the code names, and `_uncertain`) and
/// where the reason's chain starts, if it has one ([`Hit::origin`]).
pub(crate) type Verdict = (String, Option<(String, Option<String>)>);

/// The options of [`Static::decide`].
#[derive(Copy, Clone, Debug, Default)]
pub(crate) struct Options {
    /// `--strict`.
    pub(crate) strict: bool,
    /// `--follow`.
    pub(crate) follow: Option<Follow>,
    /// Compute the relation even when the change reaches no route
    /// (measurements).
    pub(crate) force_follow: bool,
}

/// The apps of an end-to-end selection from source.
#[derive(Debug)]
pub(crate) struct Static {
    root: PathBuf,
    apps: Vec<App>,
    /// Playwright config directory (root-relative) → the apps its specs can
    /// reach.
    by_config: BTreeMap<String, Vec<usize>>,
    pub(crate) notes: Vec<String>,
}

/// Why a route is reached by the change.
#[derive(Clone, Debug)]
pub(crate) struct Hit {
    /// The affected entry file of the route (root-relative).
    pub(crate) entry: String,
    /// The chain from the changed node to the entry file, both inclusive.
    pub(crate) chain: Vec<String>,
    /// The chain as lines: what changed and how it concerns the first node,
    /// then one line per hop (`→ imported by …`).
    pub(crate) steps: Vec<String>,
    /// How the change set seeds the first node: `modified`, `deleted`,
    /// `package`, `probe`, `membership`, `scope`, `named`; `uncertain` for
    /// the uncertainty policy.
    pub(crate) cause: &'static str,
    /// What changed: the first node of the chain when it was modified or
    /// removed, else the path that the first node names, lists or looks
    /// for, or `package <name>`. `None` when the entry is selected by the
    /// uncertainty policy: the first node is then the code with unknown
    /// dependencies, and no changed file is on the chain.
    pub(crate) changed: Option<String>,
    /// Set when the entry is selected by the uncertainty policy, not by the
    /// change.
    pub(crate) uncertain: Option<String>,
    /// With `uncertain`: the file with the code whose dependencies are not
    /// known.
    pub(crate) site: Option<String>,
    /// With `uncertain`, for a module load with an unknown target that the
    /// declared packages bound (`crate::loads`): why this change selects
    /// the entry through it. Such an entry is not selected on every change.
    pub(crate) bound: Option<String>,
    /// Only on a soft navigation (an intercepting route).
    pub(crate) soft: bool,
}

impl Hit {
    /// Where the chain starts: `Ok` what changed ([`Hit::changed`]), `Err` the
    /// code with unknown dependencies that selects the entry without a
    /// changed file on the chain.
    pub(crate) fn origin(&self) -> Result<&str, &str> {
        match &self.changed {
            Some(c) => Ok(c),
            None => Err(self
                .site
                .as_deref()
                .or(self.chain.first().map(String::as_str))
                .unwrap_or("")),
        }
    }
}

/// How a spec reaches a route.
#[derive(Clone, Debug)]
pub(crate) enum How {
    /// The spec names a URL.
    Named {
        /// `file:line`.
        at: String,
        url: String,
        /// The Playwright call.
        call: &'static str,
        /// Rules applied on the way (`redirect:/old`, `microfrontend:...`).
        via: Vec<String>,
    },
    /// A route the spec reaches leads there.
    Via {
        /// `(app, route)` it comes from.
        from: (usize, usize),
        kind: InvokeKind,
        /// `file:line` of the site.
        at: String,
    },
    /// The middleware of the app can send a request there.
    Middleware {
        /// `file:line` of the redirect or rewrite.
        at: String,
    },
}

/// What a spec can reach under one [`Follow`].
#[derive(Clone, Debug, Default)]
pub(crate) struct Reach {
    /// `(app, route)` → how, and whether only as a soft navigation.
    pub(crate) routes: BTreeMap<(usize, usize), (How, bool)>,
    /// Why it may visit any route of its apps (empty: bounded).
    pub(crate) open: Vec<String>,
    /// Why it may call any route handler of its apps.
    pub(crate) open_handlers: Vec<String>,
}

/// The static facts of one spec.
#[derive(Clone, Debug, Default)]
pub(crate) struct SpecStatic {
    pub(crate) apps: Vec<usize>,
    /// Navigation calls whose URL is known / unknown.
    pub(crate) resolved: usize,
    pub(crate) unresolved: Vec<String>,
    /// Under each rule (`Named` always; the others when the relation was
    /// computed).
    pub(crate) reach: BTreeMap<Follow, Reach>,
    /// URLs that leave the apps (another origin, a microfrontend outside the
    /// repository).
    pub(crate) external: Vec<String>,
    /// The routes its URLs can match when every unknown part can be any string:
    /// the base of `Links`. `reach[Named]` is the base of the rules with
    /// assumptions.
    sound: Reach,
    /// The URLs it names, per app: for the prefixes a middleware can put in
    /// front.
    urls: Vec<NamedUrl>,
    /// The apps of the URLs of `urls` that a middleware can move.
    url_apps: Vec<usize>,
    /// Whether `urls` is filled in. Facts read from the store
    /// (crate::e2e_cache) come without the URLs; they are extracted again
    /// when following needs them.
    urls_known: bool,
    /// What only `--strict` counts: unread rules of a config, a middleware
    /// target that is not known. Without it they are assumed not to move a
    /// named URL to another page.
    pub(crate) assumed: BTreeSet<String>,
}

/// The Playwright calls of [`How::Named`], for the stored form.
const CALLS: [&str; 7] = [
    "page.goto",
    "a page object goes to",
    "a URL assertion on",
    "page.route",
    "a request to",
    "a click on a link to",
    "test.use({ routes })",
];

/// [`How`] as it is stored (crate::e2e_cache).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum StoredHow {
    Named {
        at: String,
        url: String,
        /// Index into [`CALLS`].
        call: u8,
        via: Vec<String>,
    },
    Via {
        app: usize,
        route: usize,
        kind: InvokeKind,
        at: String,
    },
    Middleware {
        at: String,
    },
}

impl How {
    fn stored(&self) -> StoredHow {
        match self {
            How::Named { at, url, call, via } => StoredHow::Named {
                at: at.clone(),
                url: url.clone(),
                call: CALLS.iter().position(|c| c == call).unwrap_or(0) as u8,
                via: via.clone(),
            },
            How::Via { from, kind, at } => StoredHow::Via {
                app: from.0,
                route: from.1,
                kind: *kind,
                at: at.clone(),
            },
            How::Middleware { at } => StoredHow::Middleware { at: at.clone() },
        }
    }

    fn restored(s: &StoredHow) -> How {
        match s {
            StoredHow::Named { at, url, call, via } => How::Named {
                at: at.clone(),
                url: url.clone(),
                call: CALLS.get(*call as usize).copied().unwrap_or(CALLS[0]),
                via: via.clone(),
            },
            StoredHow::Via {
                app,
                route,
                kind,
                at,
            } => How::Via {
                from: (*app, *route),
                kind: *kind,
                at: at.clone(),
            },
            StoredHow::Middleware { at } => How::Middleware { at: at.clone() },
        }
    }
}

/// [`Reach`] as it is stored.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct StoredReach {
    /// `(app, route, how, only soft)`.
    routes: Vec<(usize, usize, StoredHow, bool)>,
    open: Vec<String>,
    open_handlers: Vec<String>,
}

impl Reach {
    fn stored(&self) -> StoredReach {
        StoredReach {
            routes: self
                .routes
                .iter()
                .map(|(&(a, r), (how, soft))| (a, r, how.stored(), *soft))
                .collect(),
            open: self.open.clone(),
            open_handlers: self.open_handlers.clone(),
        }
    }

    fn restored(s: &StoredReach) -> Reach {
        Reach {
            routes: s
                .routes
                .iter()
                .map(|(a, r, how, soft)| ((*a, *r), (How::restored(how), *soft)))
                .collect(),
            open: s.open.clone(),
            open_handlers: s.open_handlers.clone(),
        }
    }
}

/// What following adds to a spec in one app under one rule
/// ([`Static::follow_app`]): the routes of the app the spec reaches, complete
/// (those its URLs name included), and what the walk reports, in the order it
/// does.
#[derive(Clone, Debug, Default)]
struct Followed {
    /// `(route, how, only soft)`, by route.
    routes: Vec<(usize, How, bool)>,
    /// Why it may visit any route.
    open: Vec<String>,
    /// Why it may call any handler.
    open_handlers: Vec<String>,
    /// What the bounded rules assume away.
    assumed: Vec<String>,
}

/// [`Followed`] as it is stored, with its key.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct StoredFollowed {
    app: usize,
    /// [`Follow`] as a number.
    rule: u8,
    key: String,
    routes: Vec<(usize, StoredHow, bool)>,
    open: Vec<String>,
    open_handlers: Vec<String>,
    assumed: Vec<String>,
}

/// The facts of one spec as they are stored: what its URLs name
/// ([`Static::named`], without the URLs themselves) and what following added.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct StoredSpec {
    key: String,
    resolved: usize,
    unresolved: Vec<String>,
    external: Vec<String>,
    assumed: Vec<String>,
    named: StoredReach,
    sound: StoredReach,
    url_apps: Vec<usize>,
    followed: Vec<StoredFollowed>,
}

/// Every spec's stored facts (`<cache dir>/e2e-specs`).
#[derive(Debug, Default, Serialize, Deserialize)]
struct StoredSpecs {
    version: u32,
    specs: BTreeMap<String, StoredSpec>,
}

const SPECS_FILE: &str = "e2e-specs";

/// The relation of one app as it is stored (`<cache dir>/e2e-rel-<app>`).
#[derive(Debug, Serialize, Deserialize)]
struct StoredRel {
    version: u32,
    key: String,
    rel: SharedRelation,
    middleware: Vec<Edge>,
    /// [`invoke::Prefix`]es: per segment its parts, `None` for unknown text.
    prefixes: Vec<Vec<Vec<Option<String>>>>,
}

impl SpecStatic {
    fn stored(&self, key: u128) -> StoredSpec {
        StoredSpec {
            key: e2e_cache::hex(key),
            resolved: self.resolved,
            unresolved: self.unresolved.clone(),
            external: self.external.clone(),
            assumed: self.assumed.iter().cloned().collect(),
            named: self.reach[&Follow::Named].stored(),
            sound: self.sound.stored(),
            url_apps: self.url_apps.clone(),
            followed: Vec::new(),
        }
    }

    fn restored(s: &StoredSpec, apps: Vec<usize>) -> SpecStatic {
        let mut out = SpecStatic {
            apps,
            resolved: s.resolved,
            unresolved: s.unresolved.clone(),
            external: s.external.clone(),
            sound: Reach::restored(&s.sound),
            assumed: s.assumed.iter().cloned().collect(),
            url_apps: s.url_apps.clone(),
            urls_known: false,
            ..SpecStatic::default()
        };
        out.reach.insert(Follow::Named, Reach::restored(&s.named));
        out
    }

    /// The reach a rule starts from: what the URLs can match for `Links`, what
    /// they name for the rules with assumptions.
    fn base(&self, rule: Follow) -> &Reach {
        if rule >= Follow::Links {
            &self.sound
        } else {
            &self.reach[&Follow::Named]
        }
    }

    /// Whether following in app `ai` under `rule` has anything to start from.
    fn starts_in(&self, ai: usize, rule: Follow) -> bool {
        self.url_apps.contains(&ai)
            || self
                .base(rule)
                .routes
                .range((ai, 0)..=(ai, usize::MAX))
                .next()
                .is_some()
    }
}

/// A URL a spec names, as matched against one app.
#[derive(Clone, Debug)]
struct NamedUrl {
    app: usize,
    url: turborepo_trace_frameworks::nextjs::urlpat::UrlPat,
    soft: bool,
    how: How,
    /// The app's middleware runs for it.
    middleware: bool,
}

/// Why a spec runs, through the routes.
#[derive(Clone, Debug)]
pub(crate) enum Why {
    /// A route it reaches is reached by the change.
    Route {
        app: String,
        pattern: String,
        hit: Hit,
        /// The steps from the spec to the route, the spec's line first.
        path: Vec<String>,
    },
    /// It may visit any route, and the change reaches one.
    AnyRoute {
        app: String,
        open: String,
        pattern: String,
        hit: Hit,
    },
    /// An input of every route of an app changed.
    Global {
        app: String,
        what: &'static str,
        chain: Vec<String>,
        steps: Vec<String>,
        /// [`Hit::changed`].
        changed: Option<String>,
    },
    /// A route file was removed.
    Removed { app: String, path: String },
}

impl Why {
    fn chain_text(steps: &[String]) -> String {
        steps.join("\n    ")
    }

    /// The reason as text: the chain from the changed module to the spec's
    /// line.
    pub(crate) fn text(&self) -> String {
        match self {
            Why::Route {
                pattern, hit, path, ..
            } => {
                let mut s = match (&hit.uncertain, &hit.bound) {
                    // A load that the declared packages bound: this change, not any change.
                    (Some(u), Some(b)) => format!(
                        "{} is an entry file of {pattern} and reaches a module load whose target \
                         is not known ({u}); {b}",
                        hit.entry
                    ),
                    (Some(u), None) => format!(
                        "{} is an entry file of {pattern} and reaches code whose dependencies \
                         turbo trace cannot determine ({u}), so any change may affect it",
                        hit.entry
                    ),
                    (None, _) => format!(
                        "{}\n    → an entry file of {pattern}{}",
                        Self::chain_text(&hit.steps),
                        if hit.soft {
                            " (rendered when the route is reached by a client navigation)"
                        } else {
                            ""
                        }
                    ),
                };
                for step in path {
                    s.push_str(&format!("\n    → {step}"));
                }
                s
            }
            Why::AnyRoute {
                open, pattern, hit, ..
            } => format!(
                "it may visit any route ({open}), and the change reaches {pattern}:\n    {}",
                Self::chain_text(&hit.steps).replace("\n    ", "\n      ")
            ),
            Why::Global {
                app, what, steps, ..
            } => format!(
                "{}\n    → {what} of {app}: every route of the app",
                Self::chain_text(steps)
            ),
            Why::Removed { app, path } => {
                format!("{path} was removed: a route file or an input of every route of {app}")
            }
        }
    }

    pub(crate) fn value(&self) -> serde_json::Value {
        match self {
            Why::Route {
                app,
                pattern,
                hit,
                path,
            } => json!({
                "kind": match (&hit.uncertain, &hit.bound) {
                    (Some(_), Some(_)) => "route_load",
                    (Some(_), None) => "route_uncertain",
                    (None, _) => "route",
                },
                "app": app, "route": pattern, "entry": hit.entry, "chain": hit.chain,
                "cause": hit.cause, "changed": hit.changed, "site": hit.site,
                "soft": hit.soft, "visit": path,
            }),
            Why::AnyRoute {
                app,
                open,
                pattern,
                hit,
            } => json!({
                "kind": "any_route", "app": app, "open": open, "route": pattern,
                "entry": hit.entry, "chain": hit.chain,
                "cause": hit.cause, "changed": hit.changed, "site": hit.site,
            }),
            Why::Global {
                app,
                what,
                chain,
                changed,
                ..
            } => json!({
                "kind": "app_input", "app": app, "what": what, "chain": chain,
                "changed": changed,
            }),
            Why::Removed { app, path } => json!({
                "kind": "route_removed", "app": app, "path": path,
            }),
        }
    }
}

/// What the change reaches in one app.
#[derive(Debug, Default)]
pub(crate) struct AppChange {
    /// Per route: the first hit.
    pub(crate) routes: Vec<Option<Hit>>,
    /// An input of every route: what it is, and the hit.
    pub(crate) global: Option<(&'static str, Hit)>,
    /// Removed route files.
    pub(crate) removed: Vec<String>,
    /// Entry files that are not in the graph.
    pub(crate) untraced: usize,
    /// Build-time configs the change reaches only through a computed path or a
    /// listing: not counted without `--strict`.
    pub(crate) not_counted: Vec<String>,
}

impl AppChange {
    fn any(&self) -> Option<(usize, &Hit)> {
        // A route reached on a hard navigation first.
        self.routes
            .iter()
            .enumerate()
            .filter_map(|(i, h)| h.as_ref().map(|h| (i, h)))
            .min_by_key(|(_, h)| (h.soft, h.uncertain.is_some()))
    }
}

/// The decided state: what the change reaches, and each spec's reach.
pub(crate) struct Decided {
    pub(crate) changes: Vec<AppChange>,
    /// Whether the invocation relation was computed.
    pub(crate) followed: bool,
    /// The rule in force.
    pub(crate) rule: Follow,
    pub(crate) specs: BTreeMap<String, SpecStatic>,
    pub(crate) timings: Vec<(&'static str, u128)>,
}

impl Static {
    /// Loads the apps the specs of the Playwright configs in `config_dirs`
    /// (root-relative) can reach.
    pub(crate) fn load(root: &Path, config_dirs: &BTreeSet<String>) -> Static {
        let mut st = Static {
            root: root.to_path_buf(),
            apps: Vec::new(),
            by_config: BTreeMap::new(),
            notes: Vec::new(),
        };
        let mut all: Option<Vec<String>> = None;
        let mut every = |root: &Path| -> Vec<String> {
            all.get_or_insert_with(|| {
                let mut v: Vec<String> = detect(root).iter().map(|d| rel(root, d)).collect();
                v.sort();
                v
            })
            .clone()
        };
        let mut per_config: Vec<(String, Vec<String>)> = Vec::new();
        for dir in config_dirs {
            // The app the config is in; else the apps below it; else every app.
            let mut dirs: Vec<String> = Vec::new();
            let mut d = dir.as_str();
            loop {
                if is_next_app(&root.join(d)) {
                    dirs.push(d.to_string());
                    break;
                }
                match d.rsplit_once('/') {
                    Some((up, _)) => d = up,
                    None if !d.is_empty() => d = "",
                    None => break,
                }
            }
            if dirs.is_empty() {
                let prefix = if dir.is_empty() {
                    String::new()
                } else {
                    format!("{dir}/")
                };
                dirs = every(root)
                    .into_iter()
                    .filter(|a| a.starts_with(&prefix))
                    .collect();
            }
            if dirs.is_empty() {
                dirs = every(root);
                if !dirs.is_empty() {
                    st.notes.push(format!(
                        "{}: its Playwright config is in no Next.js app and has none below it: \
                         its specs are matched against every app under the root",
                        if dir.is_empty() { "." } else { dir }
                    ));
                }
            }
            per_config.push((dir.clone(), dirs));
        }
        // The apps of the configs, loaded side by side and kept in the order the
        // configs name them.
        let mut wanted: Vec<&str> = Vec::new();
        for a in per_config.iter().flat_map(|(_, dirs)| dirs) {
            if !wanted.contains(&a.as_str()) {
                wanted.push(a);
            }
        }
        let roots: Vec<PathBuf> = wanted.iter().map(|a| root.join(a)).collect();
        for (a, next) in wanted
            .iter()
            .zip(turborepo_trace_frameworks::nextjs::load_apps(&roots))
        {
            st.add_app(a, next);
        }
        for (dir, dirs) in &per_config {
            let idx: Vec<usize> = dirs.iter().map(|a| st.app(a)).collect();
            st.by_config.insert(dir.clone(), idx);
        }
        // Apps that serve some of a loaded app's URLs (microfrontends).
        let mut i = 0;
        while i < st.apps.len() {
            let names: Vec<String> = st.apps[i]
                .next
                .mfe
                .others
                .iter()
                .map(|o| o.app.clone())
                .collect();
            for name in names {
                if st.app_named(&name).is_some() {
                    continue;
                }
                let found = every(root).into_iter().find(|d| {
                    let leaf = d.rsplit('/').next().unwrap_or(d);
                    leaf == name || package_name(&root.join(d)).as_deref() == Some(name.as_str())
                });
                if let Some(d) = found {
                    st.app(&d);
                }
            }
            i += 1;
        }
        st
    }

    fn app(&mut self, dir: &str) -> usize {
        if let Some(i) = self.apps.iter().position(|a| a.dir == dir) {
            return i;
        }
        let next = NextApp::load(&self.root.join(dir));
        self.add_app(dir, next)
    }

    fn add_app(&mut self, dir: &str, next: NextApp) -> usize {
        let mut app = App {
            dir: dir.to_string(),
            next,
            files: Vec::new(),
            globals: Vec::new(),
            by_key: BTreeMap::new(),
            by_id: BTreeMap::new(),
        };
        for (i, r) in app.next.table.routes.iter().enumerate() {
            let c = app.next.table.segment_closure(r);
            app.files.push((
                c.files.iter().map(|f| app.in_root(f)).collect(),
                c.soft.iter().map(|f| app.in_root(f)).collect(),
            ));
            app.by_key.insert(invoke::route_key(r), i);
            app.by_id.insert(r.id.clone(), i);
        }
        for g in &app.next.globals.files {
            if let Some((what, entry)) = global_what(g.why) {
                app.globals.push(Global {
                    path: rel(&self.root, &g.file),
                    what,
                    code: g.code || entry,
                    build_time: matches!(
                        g.why,
                        GlobalWhy::NextConfig
                            | GlobalWhy::NextConfigHelper
                            | GlobalWhy::TailwindConfig
                            | GlobalWhy::PostCss
                    ),
                });
            }
        }
        if turborepo_trace_core::debug::var_os("E2E_DEBUG").is_some() {
            for u in &app.next.config.uncertainties {
                eprintln!(
                    "e2e debug: {}: config {} [{}]: {}",
                    app.label(),
                    u.field.as_deref().unwrap_or("-"),
                    u.kind.as_str(),
                    u.detail.chars().take(240).collect::<String>()
                );
            }
        }
        self.apps.push(app);
        self.apps.len() - 1
    }

    fn app_named(&self, name: &str) -> Option<usize> {
        self.apps.iter().position(|a| {
            a.next.mfe.this_app.as_deref() == Some(name)
                || a.next.package_name.as_deref() == Some(name)
                || a.dir.rsplit('/').next() == Some(name)
        })
    }

    /// The directories of the apps the specs of a Playwright config can reach.
    pub(crate) fn app_dirs_of(&self, config_dir: &str) -> Vec<&str> {
        self.by_config
            .get(config_dir)
            .map(|v| v.iter().map(|&i| self.apps[i].dir.as_str()).collect())
            .unwrap_or_default()
    }

    /// Whether there is an app to select by.
    pub(crate) fn is_empty(&self) -> bool {
        self.apps.is_empty()
    }

    /// The `turbotrace.toml` files (the root's, the apps') whose `[nextjs]`
    /// table cannot be read, as `<file>: <error>`. The fetch wrappers they
    /// declare would be missing from the relation between pages and
    /// handlers.
    pub(crate) fn url_api_errors(&self) -> Vec<String> {
        let roots: Vec<&Path> = self.apps.iter().map(|a| a.next.root.as_path()).collect();
        invoke::url_apis(&self.root, &roots).1
    }

    /// The entries to evaluate with the specs: every route's entry files and
    /// the code inputs of every route, as absolute paths.
    pub(crate) fn entries(&self) -> Vec<String> {
        let mut out: BTreeSet<&str> = BTreeSet::new();
        for a in &self.apps {
            for (hard, soft) in &a.files {
                out.extend(hard.iter().chain(soft).map(String::as_str));
            }
            out.extend(a.globals.iter().filter(|g| g.code).map(|g| g.path.as_str()));
        }
        out.into_iter()
            .map(|p| self.root.join(p).to_string_lossy().into_owned())
            .collect()
    }

    /// Route code runs in the app directory (`next dev`, `next start` and `next
    /// build` are started there), so that is where its
    /// `process.cwd()`-relative references resolve.
    pub(crate) fn entry_cwds(&self, g: &Graph) -> Vec<(NodeId, String)> {
        let mut out = Vec::new();
        for a in &self.apps {
            let mut paths: BTreeSet<&str> = BTreeSet::new();
            for (hard, soft) in &a.files {
                paths.extend(hard.iter().chain(soft).map(String::as_str));
            }
            paths.extend(a.globals.iter().filter(|g| g.code).map(|g| g.path.as_str()));
            for p in paths {
                if let Some(n) = g.node_by_path(p) {
                    out.push((n, a.dir.clone()));
                }
            }
        }
        out
    }

    /// What the change reaches in each app.
    fn changes(
        &self,
        g: &Graph,
        report: &AffectedReport,
        changes: &[Change],
        strict: bool,
    ) -> Vec<AppChange> {
        let by_path: BTreeMap<&str, &EntryResult> = report
            .entries
            .iter()
            .filter(|e| !whatif_skips(e))
            .map(|e| (g.path(e.entry), e))
            .collect();
        let changed: BTreeMap<&str, ChangeKind> =
            changes.iter().map(|c| (c.path.as_str(), c.kind)).collect();
        let own = |path: &str, soft: bool| Hit {
            entry: path.to_string(),
            chain: vec![path.to_string()],
            steps: vec![format!("{path} changed")],
            cause: "modified",
            changed: Some(path.to_string()),
            uncertain: None,
            site: None,
            bound: None,
            soft,
        };
        let hit = |path: &str, soft: bool| -> Option<Hit> {
            if let Some(e) = by_path.get(path) {
                let chain: Vec<String> = e.witness.iter().map(|&n| g.path(n).to_string()).collect();
                let uncertain = e.uncertain.as_ref().map(|u| {
                    u.reasons
                        .first()
                        .map(|(kind, line, _)| format!("{}:{line}: {kind:?}", g.path(u.node)))
                        .unwrap_or_else(|| g.path(u.node).to_string())
                });
                let first = chain.first().map_or("", String::as_str);
                let changed = match &e.cause {
                    _ if uncertain.is_some() => None,
                    Some(
                        Cause::Membership { path, .. }
                        | Cause::Scope { path, .. }
                        | Cause::Named { path, .. },
                    ) => Some(path.clone()),
                    Some(Cause::Probe { added, .. }) => Some(added.clone()),
                    Some(Cause::Package { package }) => {
                        Some(format!("package {}", g.package_info(*package).name))
                    }
                    Some(Cause::Modified { .. } | Cause::Deleted { .. }) | None => {
                        Some(first.to_string())
                    }
                };
                let (cause, head) = match &e.cause {
                    _ if uncertain.is_some() => ("uncertain", first.to_string()),
                    Some(Cause::Modified { path }) => ("modified", format!("{path} changed")),
                    Some(Cause::Deleted { path }) => ("deleted", format!("{path} was removed")),
                    Some(Cause::Probe { added, .. }) => (
                        "probe",
                        format!("{added} was added, where {first} looks for a file"),
                    ),
                    Some(Cause::Membership {
                        path,
                        kind,
                        dir,
                        pattern,
                    }) => (
                        "membership",
                        format!(
                            "{path} was {} in {dir}/{pattern}, which {first} lists",
                            kind.as_str()
                        ),
                    ),
                    Some(Cause::Package { package }) => (
                        "package",
                        format!("the package {} changed", g.package_info(*package).name),
                    ),
                    Some(Cause::Scope {
                        path, dir, pattern, ..
                    }) => (
                        "scope",
                        format!(
                            "{path} changed, inside {dir}/{pattern}, where {first} reads a path \
                             it computes"
                        ),
                    ),
                    Some(Cause::Named { path, by, site, .. }) => (
                        "named",
                        format!(
                            "{path} changed, which {by} in {first} names ({site} reads a path it \
                             computes)"
                        ),
                    ),
                    None => ("modified", format!("{first} changed")),
                };
                let mut steps = vec![head];
                for w in e.witness.windows(2) {
                    steps.push(format!("→ {} {}", hop(g, w[1], w[0]), g.path(w[1])));
                }
                return Some(Hit {
                    entry: path.to_string(),
                    chain,
                    steps,
                    cause,
                    changed,
                    uncertain,
                    site: e.uncertain.as_ref().map(|u| g.path(u.node).to_string()),
                    bound: e.uncertain.as_ref().and_then(|u| u.bound.clone()),
                    soft,
                });
            }
            // An entry file that is itself in the change set (added, or not a traced file).
            changed.contains_key(path).then(|| own(path, soft))
        };
        // A build-time config counts through code it loads and files it names.
        let lock_or_manifest = changes.iter().any(|c| {
            matches!(
                c.path.rsplit('/').next().unwrap_or(""),
                "package.json" | "pnpm-lock.yaml" | "package-lock.json" | "yarn.lock" | "bun.lock"
            )
        });
        let config_hit = |a: &App, gl: &Global, h: Hit| -> Result<Hit, String> {
            if strict || !gl.build_time || h.uncertain.is_some() {
                return Ok(h);
            }
            // A file under a router directory: the config that reads those reads the route
            // table, which the entries and removed files stand for.
            let routed = |p: &str| {
                [&a.next.table.app_dir, &a.next.table.pages_dir]
                    .into_iter()
                    .flatten()
                    .any(|d| p.starts_with(&format!("{}/", a.in_root(d))))
            };
            // Module loads anywhere on the chain; a read only of the changed file itself:
            // what a file imports is not part of its bytes.
            let hop_ok = |from: NodeId, to: NodeId, last: bool| {
                g.out_edges(from).any(|e| {
                    e.to == Some(to)
                        && !e.flags().contains(RefFlags::TYPE_ONLY)
                        && (loads(e.kind()) || (last && reads(e.kind()) && !routed(g.path(to))))
                })
            };
            let direct = matches!(h.cause, "modified" | "deleted" | "package")
                && h.chain.windows(2).enumerate().all(|(i, w)| {
                    match (g.node_by_path(&w[1]), g.node_by_path(&w[0])) {
                        (Some(from), Some(to)) => hop_ok(from, to, i == 0),
                        _ => false,
                    }
                });
            if direct || lock_or_manifest {
                return Ok(h);
            }
            // The shortest chain is not the only one: the files the config loads or names.
            if let Some(n) = g.node_by_path(&gl.path) {
                let mut seen: BTreeSet<u32> = BTreeSet::from([n.0]);
                let mut stack = vec![(n, vec![gl.path.clone()])];
                while let Some((x, path)) = stack.pop() {
                    let hit_here = |chain_from_config: &[String]| {
                        let mut chain = chain_from_config.to_vec();
                        chain.reverse();
                        let mut steps = vec![format!("{} changed", chain[0])];
                        for w in chain.windows(2) {
                            steps.push(format!("→ used by {}", w[1]));
                        }
                        Hit {
                            entry: gl.path.clone(),
                            changed: chain.first().cloned(),
                            chain,
                            steps,
                            cause: "modified",
                            uncertain: None,
                            site: None,
                            bound: None,
                            soft: false,
                        }
                    };
                    if changed.contains_key(g.path(x)) {
                        return Ok(hit_here(&path));
                    }
                    for e in g.out_edges(x) {
                        let Some(to) = e.to else { continue };
                        if g.is_external(to) || e.flags().contains(RefFlags::TYPE_ONLY) {
                            continue;
                        }
                        if loads(e.kind()) {
                            if seen.insert(to.0) {
                                let mut p = path.clone();
                                p.push(g.path(to).to_string());
                                stack.push((to, p));
                            }
                        } else if reads(e.kind())
                            && changed.contains_key(g.path(to))
                            && !routed(g.path(to))
                        {
                            let mut p = path.clone();
                            p.push(g.path(to).to_string());
                            return Ok(hit_here(&p));
                        }
                    }
                }
            }
            Err(format!(
                "{} ({}) is reached only through a path it computes or a directory it lists: {}",
                gl.path, gl.what, h.steps[0]
            ))
        };
        self.apps
            .iter()
            .map(|a| {
                let mut c = AppChange::default();
                for (hard, soft) in &a.files {
                    c.untraced += hard
                        .iter()
                        .chain(soft)
                        .filter(|f| g.node_by_path(f).is_none())
                        .count();
                    // A hit by the change before one by the uncertainty policy.
                    let mut hits: Vec<Hit> = hard
                        .iter()
                        .filter_map(|f| hit(f, false))
                        .chain(soft.iter().filter_map(|f| hit(f, true)))
                        .collect();
                    hits.sort_by_key(|h| (h.soft, h.uncertain.is_some(), h.chain.len()));
                    c.routes.push(hits.into_iter().next());
                }
                for gl in &a.globals {
                    let h = if gl.code {
                        hit(&gl.path, false)
                    } else {
                        changed
                            .contains_key(gl.path.as_str())
                            .then(|| own(&gl.path, false))
                    };
                    match h.map(|h| config_hit(a, gl, h)) {
                        Some(Ok(h)) if c.global.is_none() => c.global = Some((gl.what, h)),
                        Some(Err(why)) => c.not_counted.push(why),
                        _ => {}
                    }
                }
                // A removed route file: the table of the tree no longer has its route. A
                // removed input of every route (the middleware, a config) is no entry of the
                // tree either. A file of `public/` is requested by URL: any page can.
                let (ad, pd) = (&a.next.table.app_dir, &a.next.table.pages_dir);
                for ch in changes {
                    let Some(inner) = (if a.dir.is_empty() {
                        Some(ch.path.as_str())
                    } else {
                        ch.path
                            .strip_prefix(a.dir.as_str())
                            .and_then(|p| p.strip_prefix('/'))
                    }) else {
                        continue;
                    };
                    if inner.starts_with("public/") && c.global.is_none() {
                        c.global = Some((
                            "a file served from public/, which any page can request",
                            own(&ch.path, false),
                        ));
                    }
                    if ch.kind != ChangeKind::Deleted {
                        continue;
                    }
                    let top = inner.strip_prefix("src/").unwrap_or(inner);
                    if !top.contains('/')
                        && [
                            "middleware.",
                            "proxy.",
                            "instrumentation.",
                            "instrumentation-client.",
                            "next.config.",
                            "tailwind.config.",
                            "postcss.config.",
                        ]
                        .iter()
                        .any(|p| top.starts_with(p))
                    {
                        c.removed.push(ch.path.clone());
                        continue;
                    }
                    let under = |d: &Option<String>| {
                        d.as_deref()
                            .is_some_and(|d| inner.starts_with(&format!("{d}/")))
                    };
                    let name = inner.rsplit('/').next().unwrap_or(inner);
                    let special = under(pd)
                        || (under(ad)
                            && turborepo_trace_frameworks::nextjs::routes::special_name(
                                name,
                                &a.next.table.page_exts,
                            )
                            .is_some());
                    if special {
                        c.removed.push(ch.path.clone());
                    }
                }
                c
            })
            .collect()
    }

    /// The visits of a spec as its source names them, matched against its apps.
    fn named(
        &self,
        spec: &str,
        config_dir: &str,
        hosts: &[String],
        files: &Files<'_>,
        helpers: &HelperCache,
        matchers: &[Matcher<'_>],
    ) -> SpecStatic {
        let apps: Vec<usize> = self.by_config.get(config_dir).cloned().unwrap_or_default();
        let mut out = SpecStatic {
            apps: apps.clone(),
            ..SpecStatic::default()
        };
        let sv: SpecVisits = extract_visits_cached(
            &self.root.join(spec),
            files,
            &ExtractOptions::default(),
            helpers,
        );
        let mut reach = Reach::default();
        let mut sound = Reach::default();
        let at = |f: &Path, line: u32| format!("{}:{line}", rel(&self.root, f));
        for u in &sv.unresolved {
            if u.kind.counts_for_completeness() {
                out.unresolved.push(format!(
                    "{}: the URL of `{}` is not known",
                    at(&u.file, u.line),
                    u.text.chars().take(60).collect::<String>()
                ));
            }
        }
        let replace = sv
            .annotation
            .as_ref()
            .is_some_and(|a| a.replace && !a.routes.is_empty() && a.unreadable.is_empty());
        if replace {
            out.unresolved.clear();
        }
        if let Some(a) = &sv.annotation {
            for p in &a.routes {
                let mut found = false;
                for &ai in &apps {
                    for (ri, r) in self.apps[ai].next.table.routes.iter().enumerate() {
                        if r.pattern == *p && r.intercept.is_none() {
                            found = true;
                            for r in [&mut reach, &mut sound] {
                                r.routes.entry((ai, ri)).or_insert((
                                    How::Named {
                                        at: spec.to_string(),
                                        url: p.clone(),
                                        call: "test.use({ routes })",
                                        via: Vec::new(),
                                    },
                                    false,
                                ));
                            }
                        }
                    }
                }
                if !found {
                    out.unresolved.push(format!(
                        "{spec}: the annotated route {p} is not a route of the app"
                    ));
                }
            }
        }
        let own_host = |h: &str| -> bool {
            let host = h.split("://").nth(1).unwrap_or(h).trim_start_matches("//");
            let host = host.split(['/', ':']).next().unwrap_or(host);
            matches!(host, "localhost" | "127.0.0.1" | "0.0.0.0" | "[::1]")
                || hosts.iter().any(|x| x == host)
        };
        for v in sv.visits.iter().filter(|_| !replace) {
            if turborepo_trace_core::debug::var_os("E2E_DEBUG").is_some() && !v.unknown.is_empty() {
                eprintln!(
                    "e2e debug: visit {} {}: {}",
                    spec,
                    v.url,
                    v.unknown
                        .iter()
                        .map(|(r, s)| format!("{s}={}", r.chars().take(40).collect::<String>()))
                        .collect::<Vec<_>>()
                        .join(" | ")
                );
            }
            let navigation = v.kind.counts_for_completeness();
            if navigation {
                out.resolved += 1;
            }
            let mut url = v.url.clone();
            if let Origin::Absolute(h) = &url.origin {
                if !own_host(h) {
                    out.external
                        .push(format!("{} ({})", url, at(&v.file, v.line)));
                    continue;
                }
                url.origin = Origin::Relative;
            }
            let call = match v.kind {
                turborepo_trace_frameworks::playwright::visits::VisitKind::Goto => "page.goto",
                turborepo_trace_frameworks::playwright::visits::VisitKind::PageObject => {
                    "a page object goes to"
                }
                turborepo_trace_frameworks::playwright::visits::VisitKind::UrlAssert => {
                    "a URL assertion on"
                }
                turborepo_trace_frameworks::playwright::visits::VisitKind::Route => "page.route",
                turborepo_trace_frameworks::playwright::visits::VisitKind::Request => {
                    "a request to"
                }
                turborepo_trace_frameworks::playwright::visits::VisitKind::Click => {
                    "a click on a link to"
                }
            };
            // A `page.route` pattern intercepts requests: it names what the page calls, it
            // is not a visit.
            if v.kind == turborepo_trace_frameworks::playwright::visits::VisitKind::Route {
                continue;
            }
            let mut stack: Vec<(usize, u32)> = apps.iter().map(|&a| (a, 0)).collect();
            let mut seen: BTreeSet<usize> = BTreeSet::new();
            while let Some((ai, depth)) = stack.pop() {
                if !seen.insert(ai) {
                    continue;
                }
                let app = &self.apps[ai];
                let m = matchers[ai].match_pat(&url, v.soft);
                if let Outcome::Microfrontend(name) = &m.outcome {
                    match self.app_named(name) {
                        Some(other) if depth < 4 => stack.push((other, depth + 1)),
                        _ => out.external.push(format!(
                            "{url} ({}; microfrontend {name})",
                            at(&v.file, v.line)
                        )),
                    }
                    continue;
                }
                let mut middleware = false;
                for a in &m.ambiguities {
                    match a {
                        Ambiguity::RulesIncomplete => {
                            out.assumed.insert(format!(
                                "not every redirect and rewrite of {}'s config could be read",
                                app.label()
                            ));
                        }
                        Ambiguity::Middleware => middleware = true,
                        Ambiguity::MicrofrontendFlag { app: name, .. } => {
                            if let Some(other) = self.app_named(name)
                                && depth < 4
                            {
                                stack.push((other, depth + 1));
                            }
                        }
                        _ => {}
                    }
                }
                let how = How::Named {
                    at: at(&v.file, v.line),
                    url: v.url.to_string(),
                    call,
                    via: m.via.clone(),
                };
                let add = |reach: &mut Reach, ri: usize, how: How| {
                    reach.routes.entry((ai, ri)).or_insert((how, v.soft));
                    // Reached by a hard navigation as well: not only soft.
                    if !v.soft
                        && let Some(e) = reach.routes.get_mut(&(ai, ri))
                    {
                        e.1 = false;
                    }
                };
                // An unknown part that is a name of data (a fixture, the environment, a
                // call) matches dynamic segments, by Next's precedence; any other unknown
                // can be any string, a fixed segment name included.
                let ids = if v.data_only && !m.routes.is_empty() {
                    &m.routes
                } else {
                    &m.candidates
                };
                for id in ids {
                    if let Some(&ri) = app.by_id.get(id) {
                        add(&mut reach, ri, how.clone());
                    }
                }
                for id in &m.candidates {
                    if let Some(&ri) = app.by_id.get(id) {
                        add(&mut sound, ri, how.clone());
                    }
                }
                // A middleware that runs for the path can rewrite it behind a prefix (a
                // locale, a flags code, a tenant): the routes that have the path after
                // leading dynamic segments, by Next's precedence. Not for a URL that names
                // a static route as it is: that route is what the path means.
                let names_static = ids.iter().any(|id| {
                    app.by_id
                        .get(id)
                        .is_some_and(|&ri| app.next.table.routes[ri].is_static())
                });
                // Without the assumptions: any prefix, any route the prefixed path can hit.
                // The relation maps the targets of links and redirects with the same
                // logic (`Matcher::behind_prefix`).
                if middleware {
                    for p in matchers[ai].behind_prefix(&url, v.soft) {
                        let behind = |id: &String| {
                            let ri = *app.by_id.get(id)?;
                            let mut how = how.clone();
                            if let How::Named { via, .. } = &mut how {
                                via.push(format!(
                                    "the middleware can put {} segment(s) in front of the path",
                                    p.k
                                ));
                            }
                            Some((ri, how))
                        };
                        if !names_static {
                            for (ri, how) in p.likely.iter().filter_map(behind) {
                                add(&mut reach, ri, how);
                            }
                        }
                        for (ri, how) in p.sound.iter().filter_map(behind) {
                            add(&mut sound, ri, how);
                        }
                    }
                }
                out.urls.push(NamedUrl {
                    app: ai,
                    url: url.clone(),
                    soft: v.soft,
                    how,
                    middleware: middleware && !names_static,
                });
            }
        }
        if out.resolved == 0 && !replace && out.unresolved.is_empty() && reach.routes.is_empty() {
            out.unresolved.push(format!(
                "{spec}: no navigation was found in the spec or its helpers"
            ));
        }
        reach.open = out.unresolved.clone();
        sound.open = out.unresolved.clone();
        out.reach.insert(Follow::Named, reach);
        out.sound = sound;
        out.url_apps = out
            .urls
            .iter()
            .filter(|u| u.middleware)
            .map(|u| u.app)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        out.urls_known = true;
        out
    }

    /// What following the visits of `spec` adds in the app `ai`, under `rule`.
    /// The walk never leaves an app (a target is a route of the app the
    /// site is in), so a spec's reach is what its URLs give
    /// ([`SpecStatic::base`]) with, per app whose relation is followed, the
    /// routes of that app replaced by the ones here ([`Static::compose`]).
    /// `spec` has its URLs.
    fn follow_app(
        &self,
        spec: &SpecStatic,
        ai: usize,
        rel: &AppRel,
        matcher: &Matcher<'_>,
        rule: Follow,
    ) -> Followed {
        let app = &self.apps[ai];
        let mut routes: BTreeMap<usize, (How, bool)> = spec
            .base(rule)
            .routes
            .range((ai, 0)..=(ai, usize::MAX))
            .map(|(&(_, ri), v)| (ri, v.clone()))
            .collect();
        // What the middleware puts in front of a path, as its rewrites spell it
        // (`/precomputed/<code>/<path>`).
        for u in spec.urls.iter().filter(|u| u.middleware && u.app == ai) {
            for prefix in &rel.prefixes {
                let mut prefixed = u.url.clone();
                let mut segs = prefix.clone();
                segs.extend(prefixed.segs);
                prefixed.segs = segs;
                let pm = matcher.match_pat(&prefixed, u.soft);
                for id in &pm.candidates {
                    if let Some(&ri) = app.by_id.get(id) {
                        let mut how = u.how.clone();
                        if let How::Named { via, .. } = &mut how {
                            via.push("a prefix the middleware rewrites to".to_string());
                        }
                        routes.entry(ri).or_insert((how, u.soft));
                    }
                }
            }
        }
        let mut out = Followed::default();
        let mut stack: Vec<usize> = routes.keys().copied().collect();
        // The middleware runs before every request it matches: where it can send one.
        let mut first = true;
        let push = |list: &mut Vec<String>, w: &str| {
            if !list.iter().any(|x| x == w) {
                list.push(w.to_string());
            }
        };
        while let Some(ri) = stack.pop() {
            let from_mw = std::mem::take(&mut first);
            let mut step = |e: &Edge, is_mw: bool| {
                let nav = e.kind == InvokeKind::Navigation;
                if (nav || e.wide) && rule < Follow::Links {
                    return;
                }
                // A redirect decided in code sends the visitor to another page: followed
                // when its target is one known URL (`Redirects`), or always (`Links`).
                if e.kind == InvokeKind::Redirect
                    && matches!(e.to, EdgeTo::Route(_))
                    && (rule < Follow::Redirects || (rule < Follow::Links && !e.exact))
                {
                    return;
                }
                match &e.to {
                    EdgeTo::Route(ti) => {
                        let how = if is_mw {
                            How::Middleware { at: e.at.clone() }
                        } else {
                            How::Via {
                                from: (ai, ri),
                                kind: e.kind,
                                at: e.at.clone(),
                            }
                        };
                        match routes.get_mut(ti) {
                            None => {
                                routes.insert(*ti, (how, nav));
                                stack.push(*ti);
                            }
                            Some(x) if x.1 && !nav => x.1 = false,
                            Some(_) => {}
                        }
                    }
                    // A redirect or link whose target is not known sends the visitor to
                    // some page: counted when links are followed, assumed to stay on the
                    // named pages otherwise.
                    EdgeTo::AnyPage(w) => {
                        if rule >= Follow::Links {
                            push(&mut out.open, w);
                        } else if is_mw {
                            push(&mut out.assumed, w);
                        }
                    }
                    EdgeTo::AnyHandler(w) => push(&mut out.open_handlers, w),
                }
            };
            if from_mw {
                for e in &rel.middleware {
                    step(e, true);
                }
            }
            rel.for_edges(app, ri, |e| step(e, false));
        }
        out.routes = routes.into_iter().map(|(ri, (h, s))| (ri, h, s)).collect();
        out
    }

    /// The reach of `spec` under `rule` from what its URLs give and what
    /// following added per app (`parts`: app and [`Static::follow_app`]'s
    /// result, for the apps whose relation is followed), and what the
    /// bounded rules assume away. The same as one walk over all the apps
    /// ([`Static::follow_whole`]): that walk takes the apps one after
    /// the other, the last first.
    fn compose(
        spec: &SpecStatic,
        rule: Follow,
        parts: &mut [(usize, &Followed)],
    ) -> (Reach, Vec<String>) {
        let mut out = spec.base(rule).clone();
        let mut assumed: Vec<String> = Vec::new();
        if rule >= Follow::Links {
            for a in &spec.assumed {
                out.open.push(a.clone());
            }
        }
        let push = |list: &mut Vec<String>, w: &String| {
            if !list.contains(w) {
                list.push(w.clone());
            }
        };
        parts.sort_by_key(|p| std::cmp::Reverse(p.0));
        for (ai, f) in parts.iter() {
            for (ri, how, soft) in &f.routes {
                out.routes.insert((*ai, *ri), (how.clone(), *soft));
            }
            for w in &f.open {
                push(&mut out.open, w);
            }
            for w in &f.open_handlers {
                push(&mut out.open_handlers, w);
            }
            for w in &f.assumed {
                push(&mut assumed, w);
            }
        }
        (out, assumed)
    }

    /// Follows a spec's visits along the relations of its apps in one walk. The
    /// selection uses [`Static::follow_app`] and [`Static::compose`], whose
    /// results are kept between queries; this is what they have to equal
    /// (`TURBO_TRACE_DEBUG_E2E_CHECK=1` compares).
    fn follow_whole(
        &self,
        spec: &SpecStatic,
        rels: &[Option<AppRel>],
        matchers: &[Matcher<'_>],
        rule: Follow,
    ) -> (Reach, Vec<String>) {
        let mut out = if rule >= Follow::Links {
            spec.sound.clone()
        } else {
            spec.reach[&Follow::Named].clone()
        };
        let mut assumed: Vec<String> = Vec::new();
        // What the middleware puts in front of a path, as its rewrites spell it
        // (`/precomputed/<code>/<path>`).
        for u in spec.urls.iter().filter(|u| u.middleware) {
            let Some(rel) = rels.get(u.app).and_then(Option::as_ref) else {
                continue;
            };
            let app = &self.apps[u.app];
            for prefix in &rel.prefixes {
                let mut prefixed = u.url.clone();
                let mut segs = prefix.clone();
                segs.extend(prefixed.segs);
                prefixed.segs = segs;
                let pm = matchers[u.app].match_pat(&prefixed, u.soft);
                for id in &pm.candidates {
                    if let Some(&ri) = app.by_id.get(id) {
                        let mut how = u.how.clone();
                        if let How::Named { via, .. } = &mut how {
                            via.push("a prefix the middleware rewrites to".to_string());
                        }
                        out.routes.entry((u.app, ri)).or_insert((how, u.soft));
                    }
                }
            }
        }
        if rule >= Follow::Links {
            for a in &spec.assumed {
                out.open.push(a.clone());
            }
        }
        let mut stack: Vec<(usize, usize)> = out.routes.keys().copied().collect();
        // The middleware runs before every request it matches: where it can send one.
        let mut mw_done: BTreeSet<usize> = BTreeSet::new();
        let push = |list: &mut Vec<String>, w: &str| {
            if !list.iter().any(|x| x == w) {
                list.push(w.to_string());
            }
        };
        while let Some((ai, ri)) = stack.pop() {
            let Some(rel) = rels.get(ai).and_then(Option::as_ref) else {
                continue;
            };
            let from_mw = mw_done.insert(ai);
            let mut step = |e: &Edge, is_mw: bool| {
                let nav = e.kind == InvokeKind::Navigation;
                if (nav || e.wide) && rule < Follow::Links {
                    return;
                }
                // A redirect decided in code sends the visitor to another page: followed
                // when its target is one known URL (`Redirects`), or always (`Links`).
                if e.kind == InvokeKind::Redirect
                    && matches!(e.to, EdgeTo::Route(_))
                    && (rule < Follow::Redirects || (rule < Follow::Links && !e.exact))
                {
                    return;
                }
                match &e.to {
                    EdgeTo::Route(ti) => {
                        let how = if is_mw {
                            How::Middleware { at: e.at.clone() }
                        } else {
                            How::Via {
                                from: (ai, ri),
                                kind: e.kind,
                                at: e.at.clone(),
                            }
                        };
                        match out.routes.get_mut(&(ai, *ti)) {
                            None => {
                                out.routes.insert((ai, *ti), (how, nav));
                                stack.push((ai, *ti));
                            }
                            Some(x) if x.1 && !nav => x.1 = false,
                            Some(_) => {}
                        }
                    }
                    // A redirect or link whose target is not known sends the visitor to
                    // some page: counted when links are followed (`--strict`), assumed to
                    // stay on the named pages otherwise.
                    EdgeTo::AnyPage(w) => {
                        if rule >= Follow::Links {
                            push(&mut out.open, w);
                        } else if is_mw {
                            push(&mut assumed, w);
                        }
                    }
                    EdgeTo::AnyHandler(w) => push(&mut out.open_handlers, w),
                }
            };
            if from_mw {
                for e in &rel.middleware {
                    step(e, true);
                }
            }
            rel.for_edges(&self.apps[ai], ri, |e| step(e, false));
        }
        (out, assumed)
    }

    /// Decides what the change reaches and what each spec reaches. `specs`:
    /// root-relative path, Playwright config directory, own hosts.
    pub(crate) fn decide(
        &self,
        g: &Graph,
        report: &AffectedReport,
        changes: &[Change],
        specs: &[(String, String, Vec<String>)],
        opts: &Options,
    ) -> Decided {
        let Options {
            strict,
            follow,
            force_follow,
        } = *opts;
        let t0 = std::time::Instant::now();
        let mut timings = Vec::new();
        let app_changes = self.changes(g, report, changes, strict);
        timings.push(("routes", t0.elapsed().as_millis()));
        turborepo_trace_core::timing::mark("e2e: routes reached by the change");
        let t = std::time::Instant::now();
        // What reading the code needs, built when something has to be read: the facts
        // of an unchanged tree come from the store (crate::e2e_cache).
        let imports_cell: std::cell::OnceCell<GraphImports> = std::cell::OnceCell::new();
        let imports = || imports_cell.get_or_init(|| GraphImports::from_graph(g, &self.root));
        let files_cell: std::cell::OnceCell<Files<'_>> = std::cell::OnceCell::new();
        let files = || files_cell.get_or_init(|| Files::new(imports()));
        let helpers = HelperCache::default();
        // One matcher per app: building one compiles every rule of the config.
        let matchers_cell: std::cell::OnceCell<Vec<Matcher<'_>>> = std::cell::OnceCell::new();
        let matchers = || {
            matchers_cell
                .get_or_init(|| self.apps.iter().map(|a| a.next.matcher()).collect())
                .as_slice()
        };
        let caching = e2e_cache::enabled() && g.derived_cache_path(SPECS_FILE).is_some();
        let mut stored: StoredSpecs = if caching {
            e2e_cache::read(g, SPECS_FILE)
                .filter(|s: &StoredSpecs| s.version == e2e_cache::VERSION)
                .unwrap_or_default()
        } else {
            StoredSpecs::default()
        };
        let mut dirty = false;
        let mut closures = e2e_cache::Closures::new(g);
        // Everything an app was loaded from, and all of them together: what a URL
        // matches depends on every app it can be served by.
        let app_keys: Vec<u128> = if caching {
            self.apps
                .iter()
                .map(|a| e2e_cache::digest_of(&a.next))
                .collect()
        } else {
            vec![0; self.apps.len()]
        };
        let all_apps = {
            let mut k = e2e_cache::Key::new("apps");
            for (a, key) in self.apps.iter().zip(&app_keys) {
                k.str(&a.dir).num(*key);
            }
            k.finish()
        };
        let mut spec_keys: BTreeMap<&str, u128> = BTreeMap::new();
        let mut out: BTreeMap<String, SpecStatic> = BTreeMap::new();
        for (spec, dir, hosts) in specs {
            let key = caching.then(|| self.spec_key(g, &mut closures, all_apps, spec, dir, hosts));
            let found = key.and_then(|k| {
                stored
                    .specs
                    .get(spec)
                    .filter(|c| c.key == e2e_cache::hex(k))
            });
            let s = match found {
                Some(c) => {
                    SpecStatic::restored(c, self.by_config.get(dir).cloned().unwrap_or_default())
                }
                None => {
                    let s = self.named(spec, dir, hosts, files(), &helpers, matchers());
                    if let Some(k) = key {
                        stored.specs.insert(spec.clone(), s.stored(k));
                        dirty = true;
                    }
                    s
                }
            };
            if let Some(k) = key {
                spec_keys.insert(spec.as_str(), k);
            }
            out.insert(spec.clone(), s);
        }
        if caching {
            let before = stored.specs.len();
            stored.specs.retain(|k, _| out.contains_key(k));
            dirty |= stored.specs.len() != before;
        }
        timings.push(("visits", t.elapsed().as_millis()));
        turborepo_trace_core::timing::mark("e2e: spec visits");
        // The relation is only needed when the change reaches a route some spec does
        // not already reach by name.
        let rule = Follow::rule(follow);
        let any = app_changes
            .iter()
            .any(|c| c.routes.iter().any(Option::is_some));
        let followed = force_follow || (rule > Follow::Named && any);
        // The rule in force; every rule for measurements.
        let all = force_follow || turborepo_trace_core::debug::var_os("E2E_ALL_RULES").is_some();
        let rules: Vec<Follow> = [Follow::Calls, Follow::Redirects, Follow::Links]
            .into_iter()
            .filter(|r| all || *r == rule)
            .collect();
        if followed {
            // An app's relation is needed when the change reaches a route of it and some
            // spec that can go there still visits a known set of routes. Under `Links` a
            // spec with an unread config rule may already visit any route.
            let mut needed = vec![false; self.apps.len()];
            for s in out.values() {
                let bounded = rules.iter().any(|r| {
                    if *r >= Follow::Links {
                        s.sound.open.is_empty() && s.assumed.is_empty()
                    } else {
                        s.reach[&Follow::Named].open.is_empty()
                    }
                });
                if bounded {
                    for &a in &s.apps {
                        needed[a] = true;
                    }
                }
            }
            for (a, c) in app_changes.iter().enumerate() {
                needed[a] &= force_follow || c.routes.iter().any(Option::is_some);
            }
            let t = std::time::Instant::now();
            let mut relation_ms = 0u128;
            // The keys of the relations of the apps that are needed.
            let rel_keys: Vec<Option<u128>> = (0..self.apps.len())
                .map(|a| {
                    (caching && needed[a]).then(|| self.rel_key(g, &mut closures, a, app_keys[a]))
                })
                .collect();
            let follow_key = |spec: u128, rel: u128, rule: Follow| {
                e2e_cache::hex(
                    e2e_cache::Key::new("follow")
                        .num(spec)
                        .num(rel)
                        .num(rule as u128)
                        .finish(),
                )
            };
            // What is stored, and what has to be followed: `(spec, app)`.
            let mut parts: BTreeMap<(&str, usize, Follow), Followed> = BTreeMap::new();
            let mut todo: Vec<(&str, usize)> = Vec::new();
            for (name, _, _) in specs {
                let name = name.as_str();
                let Some(s) = out.get(name) else { continue };
                for a in (0..self.apps.len()).filter(|a| needed[*a]) {
                    let mut missing = false;
                    for &r in rules.iter().filter(|r| s.starts_in(a, **r)) {
                        let found = spec_keys.get(name).zip(rel_keys[a]).and_then(|(sk, rk)| {
                            let key = follow_key(*sk, rk, r);
                            stored
                                .specs
                                .get(name)?
                                .followed
                                .iter()
                                .find(|f| f.app == a && f.rule == r as u8 && f.key == key)
                        });
                        match found {
                            Some(f) => {
                                parts.insert(
                                    (name, a, r),
                                    Followed {
                                        routes: f
                                            .routes
                                            .iter()
                                            .map(|(ri, how, soft)| (*ri, How::restored(how), *soft))
                                            .collect(),
                                        open: f.open.clone(),
                                        open_handlers: f.open_handlers.clone(),
                                        assumed: f.assumed.clone(),
                                    },
                                );
                            }
                            None => missing = true,
                        }
                    }
                    if missing {
                        todo.push((name, a));
                    }
                }
            }
            if !todo.is_empty() {
                // The relations of their apps: read back, or scanned.
                let mut rels: Vec<Option<AppRel>> = (0..self.apps.len()).map(|_| None).collect();
                for &(_, a) in &todo {
                    if rels[a].is_none() {
                        let t = std::time::Instant::now();
                        rels[a] = Some(self.relation(g, imports(), a, rel_keys[a]));
                        relation_ms += t.elapsed().as_millis();
                    }
                }
                for &(name, a) in &todo {
                    let Some(rel) = rels[a].as_ref() else {
                        continue;
                    };
                    // The URLs of a spec whose facts were read back.
                    if !out[name].urls_known
                        && let Some((_, dir, hosts)) = specs.iter().find(|x| x.0 == name)
                    {
                        let full = self.named(name, dir, hosts, files(), &helpers, matchers());
                        out.insert(name.to_string(), full);
                    }
                    let s = &out[name];
                    for &r in rules.iter().filter(|r| s.starts_in(a, **r)) {
                        if parts.contains_key(&(name, a, r)) {
                            continue;
                        }
                        let f = self.follow_app(s, a, rel, &matchers()[a], r);
                        if let Some((sk, rk)) = spec_keys.get(name).zip(rel_keys[a])
                            && let Some(c) = stored.specs.get_mut(name)
                        {
                            c.followed.retain(|x| !(x.app == a && x.rule == r as u8));
                            c.followed.push(StoredFollowed {
                                app: a,
                                rule: r as u8,
                                key: follow_key(*sk, rk, r),
                                routes: f
                                    .routes
                                    .iter()
                                    .map(|(ri, how, soft)| (*ri, how.stored(), *soft))
                                    .collect(),
                                open: f.open.clone(),
                                open_handlers: f.open_handlers.clone(),
                                assumed: f.assumed.clone(),
                            });
                            dirty = true;
                        }
                        parts.insert((name, a, r), f);
                    }
                }
            }
            let names: Vec<String> = out.keys().cloned().collect();
            for name in &names {
                let Some(s) = out.get_mut(name) else { continue };
                for &r in &rules {
                    let mut mine: Vec<(usize, &Followed)> = parts
                        .range((name.as_str(), 0, r)..=(name.as_str(), usize::MAX, r))
                        .filter(|(k, _)| k.2 == r)
                        .map(|(k, f)| (k.1, f))
                        .collect();
                    let (reach, assumed) = Self::compose(s, r, &mut mine);
                    s.reach.insert(r, reach);
                    if r < Follow::Links {
                        s.assumed.extend(assumed);
                    }
                }
            }
            timings.push(("relation", relation_ms));
            timings.push((
                "follow",
                t.elapsed().as_millis().saturating_sub(relation_ms),
            ));
            turborepo_trace_core::timing::mark("e2e: relation followed");
            if turborepo_trace_core::debug::var_os("E2E_CHECK").is_some() {
                self.check_whole(
                    g,
                    imports(),
                    files(),
                    &helpers,
                    matchers(),
                    specs,
                    &needed,
                    &rules,
                    &out,
                );
            }
        }
        if caching && dirty {
            stored.version = e2e_cache::VERSION;
            e2e_cache::write(g, SPECS_FILE, &stored);
            turborepo_trace_core::timing::mark("e2e: facts stored");
        }
        if !followed && rule >= Follow::Links {
            // Nothing is reached: the base of the rule in force stands for it.
            for s in out.values_mut() {
                let sound = s.sound.clone();
                s.reach.insert(Follow::Links, sound);
            }
        }
        let d = Decided {
            changes: app_changes,
            followed,
            rule: if followed || rule >= Follow::Links {
                rule
            } else {
                Follow::Named
            },
            specs: out,
            timings,
        };
        // Measurement tooling: what a change of each single module would reach.
        if let Some(path) = turborepo_trace_core::debug::var_os("E2E_REACH")
            && let Err(e) = self.reach_table(g, &d, Path::new(&path))
        {
            eprintln!("TURBO_TRACE_DEBUG_E2E_REACH: {e}");
        }
        d
    }

    /// Measurement tooling (`TURBO_TRACE_DEBUG_E2E_REACH=<file>`, with
    /// `TURBO_TRACE_DEBUG_E2E_FOLLOW_ALWAYS=1` for the rule in force): for
    /// every first-party file some route loads, how many routes of each app
    /// load it over module edges (imports, `require`, `import()`,
    /// re-exports; no type-only edges) and how many specs visit one
    /// of those routes. A what-if per single changed module, file-level: it
    /// does not apply the barrel and erasure rules, which need the contents
    /// of a change.
    ///
    /// One line per file and app: `file`, `app`, `routes loading it`, `routes
    /// of the app`, `specs of the app's configs visiting one`, `specs of
    /// the app's configs`.
    fn reach_table(&self, g: &Graph, d: &Decided, out: &Path) -> std::io::Result<()> {
        use std::io::Write as _;
        let n = g.node_count() as usize;
        let words = n.div_ceil(64);
        let closure = |seed: NodeId| -> Vec<u64> {
            let mut bits = vec![0u64; words];
            let mut stack = vec![seed];
            bits[seed.0 as usize / 64] |= 1 << (seed.0 % 64);
            while let Some(x) = stack.pop() {
                for e in g.out_edges(x) {
                    if let Some(to) = e.to
                        && !g.is_external(to)
                        && matches!(
                            e.kind(),
                            RefKind::Import
                                | RefKind::Require
                                | RefKind::DynamicImport
                                | RefKind::ReExport
                        )
                        && !e.flags().contains(RefFlags::TYPE_ONLY)
                    {
                        let (w, b) = (to.0 as usize / 64, 1u64 << (to.0 % 64));
                        if bits[w] & b == 0 {
                            bits[w] |= b;
                            stack.push(to);
                        }
                    }
                }
            }
            bits
        };
        let mut f = std::io::BufWriter::new(std::fs::File::create(out)?);
        let mut by_file: BTreeMap<&str, Vec<u64>> = BTreeMap::new();
        for (ai, a) in self.apps.iter().enumerate() {
            // Per route: the union over its entry files.
            let mut routes: Vec<Vec<u64>> = Vec::with_capacity(a.files.len());
            for (hard, _) in &a.files {
                let mut bits = vec![0u64; words];
                for file in hard {
                    let Some(node) = g.node_by_path(file) else {
                        continue;
                    };
                    let c = by_file
                        .entry(file.as_str())
                        .or_insert_with(|| closure(node));
                    for (x, y) in bits.iter_mut().zip(c.iter()) {
                        *x |= *y;
                    }
                }
                routes.push(bits);
            }
            let mut route_count = vec![0u32; n];
            for bits in &routes {
                for (w, &word) in bits.iter().enumerate() {
                    let mut x = word;
                    while x != 0 {
                        route_count[w * 64 + x.trailing_zeros() as usize] += 1;
                        x &= x - 1;
                    }
                }
            }
            // Per spec of a config of this app: the union over the routes it visits here.
            let mut spec_count = vec![0u32; n];
            let mut specs = 0u32;
            for s in d.specs.values() {
                if s.apps.first() != Some(&ai) {
                    continue;
                }
                specs += 1;
                let Some(reach) = s.reach.get(&d.rule) else {
                    continue;
                };
                let mut bits = vec![0u64; words];
                let open = !reach.open.is_empty();
                for (ri, r) in routes.iter().enumerate() {
                    if open || reach.routes.contains_key(&(ai, ri)) {
                        for (x, y) in bits.iter_mut().zip(r.iter()) {
                            *x |= *y;
                        }
                    }
                }
                for (w, &word) in bits.iter().enumerate() {
                    let mut x = word;
                    while x != 0 {
                        spec_count[w * 64 + x.trailing_zeros() as usize] += 1;
                        x &= x - 1;
                    }
                }
            }
            for (i, &rc) in route_count.iter().enumerate() {
                if rc > 0 {
                    writeln!(
                        f,
                        "{}\t{}\t{rc}\t{}\t{}\t{specs}",
                        g.path(NodeId(i as u32)),
                        a.label(),
                        routes.len(),
                        spec_count[i]
                    )?;
                }
            }
        }
        Ok(())
    }

    /// The key of the stored facts of a spec (see crate::e2e_cache).
    fn spec_key(
        &self,
        g: &Graph,
        closures: &mut e2e_cache::Closures<'_>,
        all_apps: u128,
        spec: &str,
        config_dir: &str,
        hosts: &[String],
    ) -> u128 {
        let mut k = e2e_cache::Key::new("spec");
        k.str(spec).str(config_dir).num(all_apps);
        for h in hosts {
            k.str(h);
        }
        // The apps of its config, in order.
        for &a in self.by_config.get(config_dir).into_iter().flatten() {
            k.num(a as u128);
        }
        match g.node_by_path(spec) {
            Some(n) => k.num(closures.digest(&[n])),
            // Not a traced file: its bytes.
            None => k.bytes(&std::fs::read(self.root.join(spec)).unwrap_or_default()),
        };
        k.finish()
    }

    /// The nodes the relation of the app `ai` starts from: the entry files of
    /// its routes and its middleware.
    fn relation_seeds(&self, g: &Graph, ai: usize) -> Vec<NodeId> {
        let a = &self.apps[ai];
        let mut seeds: Vec<NodeId> = Vec::new();
        for (hard, soft) in &a.files {
            seeds.extend(hard.iter().chain(soft).filter_map(|f| g.node_by_path(f)));
        }
        seeds.extend(
            a.next
                .middleware
                .files
                .iter()
                .filter_map(|f| g.node_by_path(&rel(&self.root, f))),
        );
        seeds.sort();
        seeds.dedup();
        seeds
    }

    /// The key of the stored relation of the app `ai`.
    fn rel_key(
        &self,
        g: &Graph,
        closures: &mut e2e_cache::Closures<'_>,
        ai: usize,
        app_key: u128,
    ) -> u128 {
        let a = &self.apps[ai];
        let mut k = e2e_cache::Key::new("relation");
        k.str(&a.dir).num(app_key);
        k.num(closures.digest(&self.relation_seeds(g, ai)));
        // The URL-API tables the scan reads.
        for f in [
            self.root.join("turbotrace.toml"),
            a.next.root.join("turbotrace.toml"),
        ] {
            k.bytes(&std::fs::read(f).unwrap_or_default());
        }
        k.finish()
    }

    /// The relation of the app `ai`: read back when it is stored under `key`,
    /// else from a scan of the code its routes load, and stored.
    fn relation(&self, g: &Graph, imports: &GraphImports, ai: usize, key: Option<u128>) -> AppRel {
        let a = &self.apps[ai];
        let file = format!(
            "e2e-rel-{:016x}",
            xxhash_rust::xxh3::xxh3_64(a.dir.as_bytes())
        );
        if let Some(k) = key
            && let Some(c) = e2e_cache::read::<StoredRel>(g, &file)
            && c.version == e2e_cache::VERSION
            && c.key == e2e_cache::hex(k)
        {
            let prefixes = c
                .prefixes
                .into_iter()
                .map(|p| {
                    p.into_iter()
                        .map(|seg| {
                            seg.into_iter()
                                .map(|part| part.map_or(Part::Any, Part::Lit))
                                .collect()
                        })
                        .collect()
                })
                .collect();
            return AppRel::new(a, c.rel, c.middleware, prefixes);
        }
        let (rel, middleware, prefixes) = self.scan_app(g, imports, ai);
        if let Some(k) = key {
            e2e_cache::write(
                g,
                &file,
                &StoredRel {
                    version: e2e_cache::VERSION,
                    key: e2e_cache::hex(k),
                    rel: rel.clone(),
                    middleware: middleware.clone(),
                    prefixes: prefixes
                        .iter()
                        .map(|p| {
                            p.iter()
                                .map(|seg| {
                                    seg.iter()
                                        .map(|part| match part {
                                            Part::Lit(t) => Some(t.clone()),
                                            Part::Any => None,
                                        })
                                        .collect()
                                })
                                .collect()
                        })
                        .collect(),
                },
            );
        }
        AppRel::new(a, rel, middleware, prefixes)
    }

    /// The invocation relation of the app `ai` from a scan of the code its
    /// routes and its middleware load, with what the middleware does: where
    /// it can send a request, and the prefixes its rewrites spell.
    fn scan_app(
        &self,
        g: &Graph,
        imports: &GraphImports,
        ai: usize,
    ) -> (SharedRelation, Vec<Edge>, Vec<invoke::Prefix>) {
        let a = &self.apps[ai];
        let code = invoke::code_closure(g, &self.root, &self.relation_seeds(g, ai));
        let scan = invoke::static_scan(&code, imports, &self.root, &[a.next.root.as_path()]);
        self.relation_from(a, g, &scan)
    }

    /// The invocation relation of every app from one scan of the code all their
    /// routes load: what [`Static::scan_app`] has to equal per app
    /// (`TURBO_TRACE_DEBUG_E2E_CHECK=1`).
    fn relations_whole(
        &self,
        g: &Graph,
        imports: &GraphImports,
        needed: &[bool],
    ) -> Vec<Option<AppRel>> {
        if !needed.iter().any(|n| *n) {
            return needed.iter().map(|_| None).collect();
        }
        let mut seeds: Vec<NodeId> = Vec::new();
        for a in self
            .apps
            .iter()
            .zip(needed)
            .filter(|(_, n)| **n)
            .map(|(a, _)| a)
        {
            for (hard, soft) in &a.files {
                seeds.extend(hard.iter().chain(soft).filter_map(|f| g.node_by_path(f)));
            }
            seeds.extend(
                a.next
                    .middleware
                    .files
                    .iter()
                    .filter_map(|f| g.node_by_path(&rel(&self.root, f))),
            );
        }
        seeds.sort();
        seeds.dedup();
        let code = invoke::code_closure(g, &self.root, &seeds);
        let roots: Vec<&Path> = self.apps.iter().map(|a| a.next.root.as_path()).collect();
        let scan = invoke::static_scan(&code, imports, &self.root, &roots);
        self.apps
            .iter()
            .zip(needed)
            .map(|(a, n)| {
                n.then(|| {
                    let (rel, middleware, prefixes) = self.relation_from(a, g, &scan);
                    AppRel::new(a, rel, middleware, prefixes)
                })
            })
            .collect()
    }

    /// The relation of `a` from a scan of at least the code its routes load,
    /// with what its middleware does: where it can send a request, and the
    /// prefixes its rewrites spell.
    fn relation_from(
        &self,
        a: &App,
        g: &Graph,
        scan: &invoke::Scan,
    ) -> (SharedRelation, Vec<Edge>, Vec<invoke::Prefix>) {
        let rel = invoke::relation_of(&a.next, g, &self.root, scan, true);
        // Rewrites and redirects of the middleware that end in an open path: the
        // segments in front of it.
        let mut prefixes: Vec<invoke::Prefix> = Vec::new();
        if let Some(mw) = &rel.middleware {
            let files: BTreeSet<&str> = rel
                .route_files(mw)
                .into_iter()
                .map(|fi| rel.files[fi as usize].file.as_str())
                .collect();
            prefixes = invoke::spelled_prefixes(
                scan.sites
                    .iter()
                    .filter(|site| files.contains(self::rel(&self.root, &site.file).as_str())),
            );
        }
        let mut middleware = Vec::new();
        if let Some(mw) = &rel.middleware {
            for inv in rel.invocations(mw) {
                let at = format!("{}:{}", inv.file, inv.line);
                match (&inv.target, inv.kind) {
                    (Target::Route(k), kind) if kind.is_dependency() => {
                        if let Some(&ti) = a.by_key.get(k) {
                            middleware.push(Edge {
                                to: EdgeTo::Route(ti),
                                kind,
                                at,
                                exact: inv.url.as_deref().is_some_and(|u| !u.contains('*')),
                                wide: inv.wide,
                            });
                        }
                    }
                    (Target::Unknown(_), InvokeKind::Redirect) => middleware.push(Edge {
                        to: EdgeTo::AnyPage(format!(
                            "{at}: the middleware of {} redirects or rewrites to a URL that is \
                             not known",
                            a.label()
                        )),
                        kind: InvokeKind::Redirect,
                        at,
                        exact: false,
                        wide: false,
                    }),
                    _ => {}
                }
            }
            if mw.open {
                middleware.push(Edge {
                    to: EdgeTo::AnyHandler(format!(
                        "the middleware of {} makes a request whose URL comes from outside the \
                         code",
                        a.label()
                    )),
                    kind: InvokeKind::Fetch,
                    at: String::new(),
                    exact: false,
                    wide: false,
                });
            }
        }
        (rel, middleware, prefixes)
    }

    /// `TURBO_TRACE_DEBUG_E2E_CHECK=1`: computes every spec's reach the way it
    /// was computed before the facts were kept per spec and app (one scan
    /// of the code of every needed app, one walk per spec over all of them)
    /// and reports on standard error where the reach in `out` differs.
    #[allow(clippy::too_many_arguments)]
    fn check_whole(
        &self,
        g: &Graph,
        imports: &GraphImports,
        files: &Files<'_>,
        helpers: &HelperCache,
        matchers: &[Matcher<'_>],
        specs: &[(String, String, Vec<String>)],
        needed: &[bool],
        rules: &[Follow],
        out: &BTreeMap<String, SpecStatic>,
    ) {
        let rels = self.relations_whole(g, imports, needed);
        let (mut compared, mut differ) = (0usize, 0usize);
        for (spec, dir, hosts) in specs {
            let mut whole = self.named(spec, dir, hosts, files, helpers, matchers);
            let Some(mine) = out.get(spec) else { continue };
            for &r in rules {
                let (reach, assumed) = self.follow_whole(&whole, &rels, matchers, r);
                compared += 1;
                let same = mine
                    .reach
                    .get(&r)
                    .is_some_and(|m| m.stored() == reach.stored());
                if !same {
                    differ += 1;
                    let m = mine.reach.get(&r);
                    eprintln!(
                        "e2e check: {spec} under {}: {} route(s), {} open, {} open handler(s) \
                         kept per app; {} route(s), {} open, {} open handler(s) in one walk",
                        r.name(),
                        m.map_or(0, |m| m.routes.len()),
                        m.map_or(0, |m| m.open.len()),
                        m.map_or(0, |m| m.open_handlers.len()),
                        reach.routes.len(),
                        reach.open.len(),
                        reach.open_handlers.len(),
                    );
                }
                if r < Follow::Links {
                    whole.assumed.extend(assumed);
                }
            }
            if whole.assumed != mine.assumed {
                differ += 1;
                eprintln!("e2e check: {spec}: what the bounded rules assume differs");
            }
        }
        eprintln!(
            "e2e check: {compared} reach(es) of {} spec(s) compared with one walk over one scan: \
             {differ} differ",
            specs.len()
        );
    }

    /// Why `spec` runs through a route, under `rule` (`None`: it does not).
    pub(crate) fn why(&self, d: &Decided, spec: &SpecStatic, rule: Follow) -> Option<Why> {
        let rule = if d.followed || spec.reach.contains_key(&rule) {
            rule
        } else {
            Follow::Named
        };
        let reach = spec.reach.get(&rule)?;
        // 1. A route it reaches.
        let mut best: Option<((usize, usize), &Hit)> = None;
        for (&(ai, ri), (_, soft)) in &reach.routes {
            let Some(h) = d.changes[ai].routes[ri].as_ref() else {
                continue;
            };
            if h.soft && !soft {
                continue;
            }
            let better = best.is_none_or(|(_, b)| {
                (h.uncertain.is_some(), h.chain.len()) < (b.uncertain.is_some(), b.chain.len())
            });
            if better {
                best = Some(((ai, ri), h));
            }
        }
        if let Some(((ai, ri), h)) = best {
            return Some(Why::Route {
                app: self.apps[ai].label().to_string(),
                pattern: self.apps[ai].next.table.routes[ri].pattern.clone(),
                hit: h.clone(),
                path: self.path(reach, (ai, ri)),
            });
        }
        for &ai in &spec.apps {
            let c = &d.changes[ai];
            let app = &self.apps[ai];
            // 2. An input of every route.
            if let Some((what, h)) = &c.global {
                return Some(Why::Global {
                    app: app.label().to_string(),
                    what,
                    chain: h.chain.clone(),
                    steps: h.steps.clone(),
                    changed: h.changed.clone(),
                });
            }
            if let Some(p) = c.removed.first() {
                return Some(Why::Removed {
                    app: app.label().to_string(),
                    path: p.clone(),
                });
            }
            // 3. It may visit any route.
            if let Some(open) = reach.open.first()
                && let Some((ri, h)) = c.any()
            {
                return Some(Why::AnyRoute {
                    app: app.label().to_string(),
                    open: open.clone(),
                    pattern: app.next.table.routes[ri].pattern.clone(),
                    hit: h.clone(),
                });
            }
            // 4. It may call any handler.
            if let Some(open) = reach.open_handlers.first() {
                let handler = c.routes.iter().enumerate().find(|(ri, h)| {
                    h.is_some() && app.next.table.routes[*ri].kind == RouteKind::Handler
                });
                if let Some((ri, Some(h))) = handler {
                    return Some(Why::AnyRoute {
                        app: app.label().to_string(),
                        open: open.clone(),
                        pattern: app.next.table.routes[ri].pattern.clone(),
                        hit: h.clone(),
                    });
                }
            }
        }
        None
    }

    /// The steps from the spec to `(app, route)`, the route's own step first.
    fn path(&self, reach: &Reach, mut at: (usize, usize)) -> Vec<String> {
        let mut out = Vec::new();
        for _ in 0..32 {
            let Some((how, _)) = reach.routes.get(&at) else {
                break;
            };
            let pattern = |k: (usize, usize)| self.apps[k.0].next.table.routes[k.1].pattern.clone();
            match how {
                How::Named {
                    at: site,
                    url,
                    call,
                    via,
                } => {
                    out.push(format!(
                        "visited at {site} ({call} {url}{})",
                        if via.is_empty() {
                            String::new()
                        } else {
                            format!(", through {}", via.join(", "))
                        }
                    ));
                    break;
                }
                How::Middleware { at: site } => {
                    out.push(format!("where the middleware can send a request ({site})"));
                    break;
                }
                How::Via {
                    from,
                    kind,
                    at: site,
                } => {
                    let verb = match kind {
                        InvokeKind::Navigation => "links to it",
                        InvokeKind::Redirect => "redirects to it",
                        InvokeKind::Action => "runs it as a server action",
                        _ => "requests it",
                    };
                    out.push(format!("{} {verb} ({site})", pattern(*from)));
                    at = *from;
                }
            }
        }
        out
    }

    /// The apps as JSON: routes, what the change reaches.
    pub(crate) fn apps_value(&self, d: &Decided) -> serde_json::Value {
        let apps: Vec<serde_json::Value> = self
            .apps
            .iter()
            .zip(&d.changes)
            .map(|(a, c)| {
                let reached: Vec<serde_json::Value> = c
                    .routes
                    .iter()
                    .enumerate()
                    .filter_map(|(i, h)| {
                        let h = h.as_ref()?;
                        let r = &a.next.table.routes[i];
                        Some(json!({
                            "route": r.pattern,
                            "kind": if r.kind == RouteKind::Page { "page" } else { "handler" },
                            "entry": h.entry,
                            "chain": h.chain,
                            "cause": h.cause,
                            "changed": h.changed,
                            "uncertain": h.uncertain,
                            "bound": h.bound,
                            "soft": h.soft,
                        }))
                    })
                    .collect();
                let pages = a
                    .next
                    .table
                    .routes
                    .iter()
                    .filter(|r| r.kind == RouteKind::Page)
                    .count();
                json!({
                    "app": a.label(),
                    "routes": a.next.table.routes.len(),
                    "pages": pages,
                    "entry_files": a.files.iter().flat_map(|(h, s)| h.iter().chain(s)).collect::<BTreeSet<_>>().len(),
                    "untraced_entry_files": c.untraced,
                    "reached": reached.len(),
                    "reached_by_uncertainty": c.routes.iter().flatten().filter(|h| h.uncertain.is_some()).count(),
                    "reached_routes": reached,
                    "every_route": c.global.as_ref().map(|(what, h)| json!({ "what": what, "chain": h.chain, "cause": h.cause, "changed": h.changed })),
                    "not_counted": c.not_counted,
                    "removed": c.removed,
                })
            })
            .collect();
        json!(apps)
    }

    /// One spec's static facts as JSON.
    pub(crate) fn spec_value(&self, d: &Decided, s: &SpecStatic) -> serde_json::Value {
        let mut by_rule = serde_json::Map::new();
        for rule in Follow::ALL {
            let Some(r) = s.reach.get(&rule) else {
                continue;
            };
            by_rule.insert(
                rule.name().to_string(),
                json!({
                    "routes": r.routes.len(),
                    "open": r.open,
                    "open_handlers": r.open_handlers,
                    "run": self.why(d, s, rule).is_some(),
                }),
            );
        }
        let rule = d.rule;
        let routes: Vec<String> = s
            .reach
            .get(&rule)
            .map(|r| {
                r.routes
                    .keys()
                    .map(|&(ai, ri)| self.apps[ai].next.table.routes[ri].pattern.clone())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect()
            })
            .unwrap_or_default();
        json!({
            "apps": s.apps.iter().map(|&a| self.apps[a].label()).collect::<Vec<_>>(),
            "navigations": { "resolved": s.resolved, "unresolved": s.unresolved },
            "external": s.external,
            "rule": rule.name(),
            "assumed": s.assumed,
            "routes": routes,
            "by_rule": by_rule,
        })
    }

    /// `--format summary`: the flow from the changed files through the routes
    /// to the specs, as text. `verdicts`: per spec, the kind of its first
    /// reason (`None`: skipped) and, for a reason through a route, the
    /// first node of its chain.
    pub(crate) fn summary_text(&self, d: &Decided, verdicts: &[Verdict]) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        let total = verdicts.len();
        let selected = verdicts.iter().filter(|v| v.1.is_some()).count();
        let pct = |a: usize, b: usize| {
            if b == 0 {
                0.0
            } else {
                100.0 * a as f64 / b as f64
            }
        };
        let _ = writeln!(
            out,
            "{selected} of {total} spec(s) selected ({:.1}%), from source (no build, no \
             recording); visits followed by: {}",
            pct(selected, total),
            d.rule.name()
        );
        let cells = 60usize;
        let full = if total == 0 {
            0
        } else {
            (selected * cells).div_ceil(total).min(cells)
        };
        let _ = writeln!(out, "  {}{}", "█".repeat(full), "░".repeat(cells - full));

        // Where the chains start: what changed (the routes whose chain starts there,
        // the specs selected through them), and apart from that the code with
        // unknown dependencies, which is in no diff.
        let mut by_file: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
        let mut by_site: BTreeMap<&str, (usize, usize, bool)> = BTreeMap::new();
        for c in &d.changes {
            let hits = c.routes.iter().flatten().map(|h| (h, 1));
            for (h, n) in hits.chain(c.global.iter().map(|(_, h)| (h, 0))) {
                match h.origin() {
                    Ok(changed) => by_file.entry(changed).or_default().0 += n,
                    Err(site) => {
                        let e = by_site.entry(site).or_default();
                        e.0 += n;
                        e.2 |= h.bound.is_some();
                    }
                }
            }
        }
        for (_, v) in verdicts {
            if let Some((kind, Some(first))) = v {
                if matches!(kind.as_str(), "route_uncertain" | "route_load") {
                    if let Some(e) = by_site.get_mut(first.as_str()) {
                        e.1 += 1;
                    }
                } else if let Some(e) = by_file.get_mut(first.as_str()) {
                    e.1 += 1;
                }
            }
        }
        let _ = writeln!(
            out,
            "\nFrom where: what changed at the start of the chain of each route and spec"
        );
        if by_file.is_empty() {
            let _ = writeln!(out, "  no changed file reaches a route");
        } else {
            let _ = writeln!(out, "  {:>7} {:>7}  changed", "routes", "specs");
            let mut rows: Vec<(&str, (usize, usize))> = by_file.into_iter().collect();
            rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
            for (f, (r, s)) in rows.iter().take(12) {
                let _ = writeln!(out, "  {r:>7} {s:>7}  {f}");
            }
            if rows.len() > 12 {
                let _ = writeln!(out, "  ... {} more", rows.len() - 12);
            }
        }
        if !by_site.is_empty() {
            let _ = writeln!(
                out,
                "\nThrough code whose dependencies are not known: where that code is (these files \
                 are not what changed)"
            );
            let _ = writeln!(out, "  {:>7} {:>7}  code", "routes", "specs");
            let mut rows: Vec<(&str, (usize, usize, bool))> = by_site.into_iter().collect();
            rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
            for (f, (r, s, bound)) in rows.iter().take(12) {
                let _ = writeln!(
                    out,
                    "  {r:>7} {s:>7}  {f}{}",
                    if *bound {
                        " (a load with an unknown target: the change affects a package its app \
                         depends on)"
                    } else {
                        " (selected on every change)"
                    }
                );
            }
            if rows.len() > 12 {
                let _ = writeln!(out, "  ... {} more", rows.len() - 12);
            }
        }

        // Routes per app, with the specs that visit each.
        let _ = writeln!(out, "\nThrough what: the routes the change reaches");
        for (ai, (a, c)) in self.apps.iter().zip(&d.changes).enumerate() {
            let reached: Vec<(usize, &Hit)> = c
                .routes
                .iter()
                .enumerate()
                .filter_map(|(i, h)| h.as_ref().map(|h| (i, h)))
                .collect();
            if reached.is_empty() && c.global.is_none() && c.removed.is_empty() {
                continue;
            }
            let _ = writeln!(
                out,
                "  {}: {} of {} route(s){}",
                a.label(),
                reached.len(),
                a.next.table.routes.len(),
                match &c.global {
                    Some((what, _)) => format!("; and {what}: every route"),
                    None => String::new(),
                }
            );
            let mut open = 0usize;
            let mut visits: BTreeMap<usize, usize> = BTreeMap::new();
            for s in d.specs.values() {
                let Some(r) = s.reach.get(&d.rule) else {
                    continue;
                };
                if !r.open.is_empty() {
                    if s.apps.contains(&ai) {
                        open += 1;
                    }
                    continue;
                }
                for &(app, ri) in r.routes.keys() {
                    if app == ai {
                        *visits.entry(ri).or_default() += 1;
                    }
                }
            }
            let mut rows: Vec<(usize, &Hit)> = reached;
            rows.sort_by_key(|(i, _)| std::cmp::Reverse(visits.get(i).copied().unwrap_or(0)));
            let _ = writeln!(
                out,
                "    {:>5}  route  ← entry file ← what changed",
                "specs"
            );
            for (i, h) in rows.iter().take(10) {
                let _ = writeln!(
                    out,
                    "    {:>5}  {}  ← {} ← {}{}",
                    visits.get(i).copied().unwrap_or(0),
                    a.next.table.routes[*i].pattern,
                    h.entry,
                    match h.origin() {
                        Ok(changed) => changed,
                        Err(site) => site,
                    },
                    match (&h.uncertain, &h.bound) {
                        (Some(_), Some(_)) => {
                            " (not changed: it has a load with an unknown target)"
                        }
                        (Some(_), None) => " (not changed: its dependencies are not known)",
                        (None, _) => "",
                    }
                );
            }
            if rows.len() > 10 {
                let _ = writeln!(out, "    ... {} more route(s)", rows.len() - 10);
            }
            if open > 0 {
                let _ = writeln!(
                    out,
                    "    + {open} spec(s) that may visit any route of this app"
                );
            }
            for n in &c.not_counted {
                let _ = writeln!(out, "    not counted (--strict counts it): {n}");
            }
        }

        // Why.
        let mut why: BTreeMap<&str, usize> = BTreeMap::new();
        for (_, v) in verdicts {
            if let Some((kind, _)) = v {
                *why.entry(kind.as_str()).or_default() += 1;
            }
        }
        fn label(k: &str) -> &str {
            match k {
                "route" => "a route it visits is reached by the change",
                "route_uncertain" => "a route it visits reaches code with unknown dependencies",
                "route_load" => {
                    "a route it visits has a load with an unknown target, and the change affects a \
                     package its app depends on"
                }
                "any_route" => "it may visit any route, and the change reaches one",
                "app_input" => "an input of every route of its app changed",
                "route_removed" => "a route file of its app was removed",
                "imports" => "its own code changed (the spec or what it imports)",
                "imports_named" => "a file changed that its own code names, lists or looks for",
                "imports_uncertain" => "its own code reaches unknown dependencies",
                "playwright_config" => {
                    "its Playwright setup changed (the config, the global setup or what they load)"
                }
                "playwright_config_named" => {
                    "a file changed that its Playwright setup names, lists or looks for"
                }
                "playwright_config_uncertain" => {
                    "its Playwright setup reaches unknown dependencies"
                }
                "snapshot" => "its screenshots changed",
                "env_file" => "an environment file changed",
                other => other,
            }
        }
        let _ = writeln!(out, "\nWhy: the first reason of each selected spec");
        let mut rows: Vec<(&str, usize)> = why.into_iter().collect();
        rows.sort_by_key(|r| std::cmp::Reverse(r.1));
        for (k, n) in rows {
            let _ = writeln!(
                out,
                "  {n:>6} {:>5.1}%  {}",
                pct(n, selected.max(1)),
                label(k)
            );
        }

        // How much of spec → routes the source decides.
        let (mut bounded, mut any) = (0usize, 0usize);
        let mut reasons: BTreeMap<String, usize> = BTreeMap::new();
        for s in d.specs.values() {
            match s.reach.get(&d.rule) {
                Some(r) if r.open.is_empty() => bounded += 1,
                Some(r) => {
                    any += 1;
                    // The shape of the first reason, without its place.
                    let w = r.open[0]
                        .split_once(": ")
                        .map_or(r.open[0].as_str(), |(_, w)| w);
                    let w = match w.find('`') {
                        Some(i) => format!(
                            "{}a URL that cannot be evaluated",
                            w[..i].replace("the URL of ", "")
                        ),
                        None => w.to_string(),
                    };
                    *reasons.entry(w).or_default() += 1;
                }
                None => {}
            }
        }
        let _ = writeln!(out, "\nWhere the specs go, from their source");
        let _ = writeln!(
            out,
            "  {bounded} spec(s) visit a known set of routes; {any} may visit any route of their \
             app{}",
            if d.followed || d.rule < Follow::Links {
                ""
            } else {
                " (by the URLs they name: links were not followed, the change reaches no route)"
            }
        );
        let mut rows: Vec<(String, usize)> = reasons.into_iter().collect();
        rows.sort_by_key(|r| std::cmp::Reverse(r.1));
        for (w, n) in rows.iter().take(5) {
            let _ = writeln!(out, "  {n:>6}  {w}");
        }
        let _ = writeln!(
            out,
            "\n  --explain <spec> prints the chain of one spec; --follow calls assumes a spec \
             stays on the pages its URLs name"
        );
        out
    }

    /// The lines of the summary on stderr: per app with a change, what it
    /// reaches.
    pub(crate) fn summary(&self, d: &Decided) -> Vec<String> {
        let mut out = Vec::new();
        for (a, c) in self.apps.iter().zip(&d.changes) {
            let reached = c.routes.iter().flatten().count();
            if reached > 0 || c.global.is_some() || !c.removed.is_empty() {
                out.push(format!(
                    "{}: the change reaches {reached} of {} route(s){}",
                    a.label(),
                    a.next.table.routes.len(),
                    match &c.global {
                        Some((what, _)) => format!(", and {what}: every route"),
                        None => String::new(),
                    }
                ));
            }
            for n in &c.not_counted {
                out.push(format!(
                    "{}: not counted (--strict counts it): {n}",
                    a.label()
                ));
            }
        }
        if out.is_empty() {
            out.push(format!(
                "the change reaches no route of {} app(s)",
                self.apps.len()
            ));
        }
        out
    }

    /// The routes a spec reaches under the rule in force, for `--explain`.
    pub(crate) fn explain_reach(&self, d: &Decided, s: &SpecStatic) -> Vec<String> {
        let rule = d.rule;
        let Some(r) = s.reach.get(&rule) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let pattern = |k: &(usize, usize)| self.apps[k.0].next.table.routes[k.1].pattern.clone();
        let mut named: Vec<String> = Vec::new();
        let mut led: BTreeSet<String> = BTreeSet::new();
        for (k, (how, _)) in &r.routes {
            match how {
                How::Named { at, .. } => named.push(format!("{} ({at})", pattern(k))),
                _ => {
                    led.insert(pattern(k));
                }
            }
        }
        let list = |v: Vec<String>| {
            let more = v.len().saturating_sub(8);
            let mut s = v.into_iter().take(8).collect::<Vec<_>>().join(", ");
            if more > 0 {
                s.push_str(&format!(", and {more} more"));
            }
            s
        };
        if named.is_empty() && led.is_empty() {
            out.push("its source names no route of the app".to_string());
        } else {
            out.push(format!(
                "its source names {} route(s): {}",
                named.len(),
                list(named)
            ));
        }
        if !led.is_empty() {
            out.push(format!(
                "{} more through what those request{}: {}",
                led.len(),
                match rule {
                    Follow::Links => ", redirect to or link to",
                    Follow::Redirects => " or redirect to",
                    _ => "",
                },
                list(led.into_iter().collect())
            ));
        }
        for o in r.open.iter().take(3) {
            out.push(format!("it may visit any route: {o}"));
        }
        for o in r.open_handlers.iter().take(1) {
            out.push(format!("it may call any route handler: {o}"));
        }
        if rule < Follow::Links {
            let mut not: Vec<String> = s.assumed.iter().take(2).cloned().collect();
            if s.assumed.len() > not.len() {
                not.push(format!("{} more", s.assumed.len() - not.len()));
            }
            out.push(format!(
                "assumed (--follow {}): it stays on these pages; their {} are not followed{}",
                rule.name(),
                match rule {
                    Follow::Redirects => "links and redirects with a computed target",
                    _ => "redirects and links",
                },
                if not.is_empty() {
                    String::new()
                } else {
                    format!(", nor: {}", not.join("; "))
                }
            ));
        }
        out
    }
}

/// Measurement tooling: `TURBO_TRACE_DEBUG_E2E_WHATIF=no-names` leaves out the
/// entries that only a name selects (a changed path named by code that is
/// loaded with a read of a computed path): what the selection would be if such
/// reads never left their package.
pub(crate) fn whatif_skips(e: &EntryResult) -> bool {
    matches!(e.cause, Some(Cause::Named { .. }))
        && turborepo_trace_core::debug::var("E2E_WHATIF").as_deref() == Ok("no-names")
}

/// A module load.
fn loads(kind: RefKind) -> bool {
    matches!(
        kind,
        RefKind::Import
            | RefKind::Require
            | RefKind::DynamicImport
            | RefKind::ReExport
            | RefKind::Worker
            | RefKind::ChildProcess
    )
}

/// A read of a file's bytes.
fn reads(kind: RefKind) -> bool {
    matches!(kind, RefKind::FsRead | RefKind::Asset | RefKind::Embed)
}

/// How `from` uses `to`, for a chain: `imported by`, `read by`, ...
fn hop(g: &Graph, from: NodeId, to: NodeId) -> &'static str {
    let mut word = "used by";
    for e in g.out_edges(from).filter(|e| e.to == Some(to)) {
        word = match e.kind() {
            RefKind::Import | RefKind::Require | RefKind::ReExport => return "imported by",
            RefKind::DynamicImport => "loaded on demand by",
            RefKind::FsRead | RefKind::Asset | RefKind::Embed => "read by",
            RefKind::FsReadDir => "listed by",
            _ => word,
        };
    }
    word
}

fn package_name(dir: &Path) -> Option<String> {
    let s = std::fs::read_to_string(dir.join("package.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&s).ok()?;
    v.get("name")?.as_str().map(String::from)
}

/// The host of a base URL (`http://localhost:3000` → `localhost`).
pub(crate) fn host_of(url: &str) -> Option<String> {
    let rest = url.split("://").nth(1)?;
    let host = rest.split(['/', ':']).next()?;
    (!host.is_empty()).then(|| host.to_string())
}
