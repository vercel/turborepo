//! The Next.js route table: App Router and Pages Router files → routes, plus
//! the segment files Next composes to render each route.
//!
//! Pure filesystem analysis (no Next.js code is loaded). Paths are relative to
//! the app root and `/`-separated.
//!
//! | Convention | Handling |
//! |---|---|
//! | route groups `(x)` | dropped from the URL, kept in the segment chain (their layouts apply) |
//! | `[x]`, `[...x]`, `[[...x]]` | [`Seg::Dynamic`], [`Seg::CatchAll`], [`Seg::OptCatchAll`] |
//! | private `_x` folders | excluded |
//! | parallel `@slot` | not part of the URL; the layout renders the slot's best-matching page, else `@slot/default` |
//! | intercepting `(.)x`, `(..)x`, `(..)(..)x`, `(...)x` | routes with [`Route::intercept`]: only rendered on soft navigation |
//! | `route.ts` | [`RouteKind::Handler`], not wrapped by layouts |
//! | metadata files (`opengraph-image.tsx`, `sitemap.ts`, `icon.png`, ...) | handler routes, and inputs of the pages below them |
//! | `pages/**`, `pages/api/**` | [`Router::Pages`]; `_app`/`_document` wrap every page |
//! | the same URL in `app/` and `pages/` | a Next build error, recorded in [`RouteTable::conflicts`] |

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use serde::Serialize;

use crate::fsx::PathFsx;

/// Default `pageExtensions`.
pub const DEFAULT_PAGE_EXTS: &[&str] = &["tsx", "ts", "jsx", "js"];

/// Special App Router file names.
pub const SPECIAL: &[&str] = &[
    "page",
    "route",
    "layout",
    "template",
    "loading",
    "error",
    "not-found",
    "forbidden",
    "unauthorized",
    "default",
    "global-error",
    "global-not-found",
];

/// Files that wrap a page at their segment (everything except
/// page/route/default and the root-only globals).
pub const BOUNDARY: &[&str] = &[
    "layout",
    "template",
    "loading",
    "error",
    "not-found",
    "forbidden",
    "unauthorized",
];

/// Pages Router special files at the top of `pages/`.
pub const PAGES_SPECIAL: &[&str] = &["_app", "_document", "_error", "404", "500"];

const METADATA_BASES: &[&str] = &[
    "opengraph-image",
    "twitter-image",
    "icon",
    "apple-icon",
    "sitemap",
    "robots",
    "manifest",
    "favicon",
];
const METADATA_STATIC_EXTS: &[&str] = &[
    "png",
    "jpg",
    "jpeg",
    "gif",
    "ico",
    "svg",
    "txt",
    "xml",
    "json",
    "webmanifest",
];

/// One URL segment of a route.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case", tag = "t", content = "v")]
pub enum Seg {
    /// A fixed segment.
    Static(String),
    /// `[x]`.
    Dynamic(String),
    /// `[...x]`: one or more segments.
    CatchAll(String),
    /// `[[...x]]`: zero or more segments.
    OptCatchAll(String),
}

impl Seg {
    /// Next's per-segment rank: static 0 < dynamic 1 < catch-all 2 < optional
    /// catch-all 3.
    pub fn rank(&self) -> u8 {
        match self {
            Seg::Static(_) => 0,
            Seg::Dynamic(_) => 1,
            Seg::CatchAll(_) => 2,
            Seg::OptCatchAll(_) => 3,
        }
    }

    fn display(&self) -> String {
        match self {
            Seg::Static(s) => s.clone(),
            Seg::Dynamic(s) => format!("[{s}]"),
            Seg::CatchAll(s) => format!("[...{s}]"),
            Seg::OptCatchAll(s) => format!("[[...{s}]]"),
        }
    }
}

/// `/a/[b]/[...c]` for a segment list.
pub fn pattern_string(segs: &[Seg]) -> String {
    let mut s = String::new();
    for x in segs {
        s.push('/');
        s.push_str(&x.display());
    }
    if s.is_empty() {
        s.push('/');
    }
    s
}

/// Parses a route pattern (`/[team]/settings`) into segments (route groups and
/// slots are dropped).
pub fn parse_pattern(p: &str) -> Vec<Seg> {
    p.split('/')
        .filter(|s| !s.is_empty())
        .filter_map(|s| match parse_dir_name(s) {
            DirKind::Url(seg) => Some(seg),
            _ => None,
        })
        .collect()
}

/// What a directory name means.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DirKind {
    /// A URL segment.
    Url(Seg),
    /// `(group)`.
    Group,
    /// `@slot`.
    Slot,
    /// `(.)x` and friends: the marker and the rest.
    Intercept(InterceptMarker, String),
    /// `_private`.
    Private,
}

/// Interception level.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InterceptMarker {
    /// `(.)`: same level.
    Same,
    /// `(..)` repeated `n` times.
    Up(u8),
    /// `(...)`: from the root.
    Root,
}

/// Classifies a directory name.
pub fn parse_dir_name(name: &str) -> DirKind {
    if let Some(rest) = name.strip_prefix("(...)") {
        return DirKind::Intercept(InterceptMarker::Root, rest.to_string());
    }
    if let Some(rest) = name.strip_prefix("(.)") {
        return DirKind::Intercept(InterceptMarker::Same, rest.to_string());
    }
    if name.starts_with("(..)") {
        let mut n = 0u8;
        let mut rest = name;
        while let Some(r) = rest.strip_prefix("(..)") {
            n += 1;
            rest = r;
        }
        return DirKind::Intercept(InterceptMarker::Up(n), rest.to_string());
    }
    if name.starts_with('(') && name.ends_with(')') {
        return DirKind::Group;
    }
    if name.starts_with('@') {
        return DirKind::Slot;
    }
    if name.starts_with('_') {
        return DirKind::Private;
    }
    if let Some(x) = name
        .strip_prefix("[[...")
        .and_then(|r| r.strip_suffix("]]"))
    {
        return DirKind::Url(Seg::OptCatchAll(x.to_string()));
    }
    if let Some(x) = name.strip_prefix("[...").and_then(|r| r.strip_suffix(']')) {
        return DirKind::Url(Seg::CatchAll(x.to_string()));
    }
    if let Some(x) = name.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
        return DirKind::Url(Seg::Dynamic(x.to_string()));
    }
    DirKind::Url(Seg::Static(name.to_string()))
}

/// Which router a route belongs to.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Router {
    /// `app/`.
    App,
    /// `pages/`.
    Pages,
}

/// Page or handler.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteKind {
    /// Renders a page.
    Page,
    /// A route handler (`route.ts`, `pages/api/**`, a metadata route).
    Handler,
}

/// An intercepting route's position.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Intercept {
    /// The marker.
    pub marker: InterceptMarker,
    /// The directory holding the intercepting folder.
    pub at: String,
}

/// One route.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Route {
    /// Stable id: `<kind>:<file>`.
    pub id: String,
    /// Router.
    pub router: Router,
    /// Page or handler.
    pub kind: RouteKind,
    /// The page/route/metadata file, relative to the app root.
    pub file: String,
    /// Its directory.
    pub dir: String,
    /// URL segments.
    pub segs: Vec<Seg>,
    /// `/a/[b]`.
    pub pattern: String,
    /// The `@slot` directory the route lives in, if any.
    pub slot: Option<String>,
    /// Set for intercepting routes.
    pub intercept: Option<Intercept>,
    /// A metadata route (`sitemap.xml`, `opengraph-image`).
    pub metadata: bool,
}

impl Route {
    /// Whether the route has only static segments.
    pub fn is_static(&self) -> bool {
        self.segs.iter().all(|s| matches!(s, Seg::Static(_)))
    }
}

/// One App Router directory.
#[derive(Clone, Debug, Default, Serialize)]
pub struct DirInfo {
    /// Path relative to the app root.
    pub rel: String,
    /// Parent directory index.
    pub parent: Option<usize>,
    /// Special files by base name.
    pub files: BTreeMap<String, String>,
    /// Child `@slot` directory indexes.
    pub slots: Vec<usize>,
    /// Metadata files.
    pub metadata: Vec<String>,
    /// The slot directory this directory is inside.
    pub slot: Option<String>,
}

/// A same-URL App/Pages pair (a Next build error).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Conflict {
    /// The URL pattern.
    pub pattern: String,
    /// Route ids.
    pub routes: Vec<String>,
}

/// The route table of one app.
#[derive(Clone, Debug, Default, Serialize)]
pub struct RouteTable {
    /// `app` or `src/app`, if present.
    pub app_dir: Option<String>,
    /// `pages` or `src/pages`, if present.
    pub pages_dir: Option<String>,
    /// Page extensions used.
    pub page_exts: Vec<String>,
    /// App Router directories (index 0 is the app dir).
    pub dirs: Vec<DirInfo>,
    /// Routes, sorted by id.
    pub routes: Vec<Route>,
    /// `pages/_app.tsx` and friends that exist.
    pub pages_special: Vec<String>,
    /// Same URL in both routers.
    pub conflicts: Vec<Conflict>,
}

/// Files a route renders: on a hard navigation, and additionally on soft
/// navigations from inside it (intercepting routes of its slots).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct SegmentClosure {
    /// Hard-navigation files (sorted).
    pub files: Vec<String>,
    /// Extra files on soft navigation (sorted, disjoint from `files`).
    pub soft: Vec<String>,
}

/// The app dir and pages dir of an app root (`app`/`src/app`,
/// `pages`/`src/pages`).
pub fn router_dirs(root: &Path) -> (Option<String>, Option<String>) {
    let pick = |names: [&str; 2]| {
        names
            .into_iter()
            .find(|n| root.join(n).fx_is_dir())
            .map(String::from)
    };
    (pick(["app", "src/app"]), pick(["pages", "src/pages"]))
}

fn rel_join(a: &str, b: &str) -> String {
    if a.is_empty() {
        b.to_string()
    } else {
        format!("{a}/{b}")
    }
}

fn list_dir(abs: &Path) -> Vec<(String, bool)> {
    crate::fsx::read_dir(abs)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(p, d)| Some((p.file_name()?.to_str()?.to_string(), d)))
        .collect()
}

/// The special base name of `file` (`page` for `page.tsx`), given page
/// extensions.
pub fn special_name<'a>(file: &'a str, exts: &[String]) -> Option<&'a str> {
    for e in exts {
        if let Some(base) = file
            .strip_suffix(e.as_str())
            .and_then(|b| b.strip_suffix('.'))
            && SPECIAL.contains(&base)
        {
            return Some(base);
        }
    }
    None
}

/// The metadata route URL segment for a metadata file name, if it is one.
fn metadata_url(file: &str, exts: &[String]) -> Option<String> {
    let (base, ext) = file.rsplit_once('.')?;
    let base_core = base.strip_suffix(".alt").unwrap_or(base);
    // Numbered variants: icon1.png, opengraph-image2.tsx.
    let trimmed = base_core.trim_end_matches(|c: char| c.is_ascii_digit());
    if !METADATA_BASES.contains(&trimmed) {
        return None;
    }
    let code = exts.iter().any(|e| e == ext);
    if code {
        return Some(match trimmed {
            "sitemap" => "sitemap.xml".into(),
            "robots" => "robots.txt".into(),
            "manifest" => "manifest.webmanifest".into(),
            _ => base_core.to_string(),
        });
    }
    METADATA_STATIC_EXTS
        .contains(&ext)
        .then(|| file.to_string())
}

impl RouteTable {
    /// Scans `root` (the app root, the directory of `next.config.*`) with the
    /// given page extensions (empty: the defaults).
    pub fn scan(root: &Path, page_exts: &[String]) -> RouteTable {
        let exts: Vec<String> = if page_exts.is_empty() {
            DEFAULT_PAGE_EXTS.iter().map(|s| s.to_string()).collect()
        } else {
            page_exts.to_vec()
        };
        let (app_dir, pages_dir) = router_dirs(root);
        let mut t = RouteTable {
            app_dir: app_dir.clone(),
            pages_dir: pages_dir.clone(),
            page_exts: exts.clone(),
            ..RouteTable::default()
        };
        if let Some(a) = &app_dir {
            t.walk_app(root, a, None, Vec::new(), None, None, 0);
        }
        if let Some(p) = &pages_dir {
            t.walk_pages(root, p, p, Vec::new(), 0);
        }
        t.routes.sort_by(|a, b| a.id.cmp(&b.id));
        t.find_conflicts();
        t
    }

    #[allow(clippy::too_many_arguments)]
    fn walk_app(
        &mut self,
        root: &Path,
        rel: &str,
        parent: Option<usize>,
        segs: Vec<Seg>,
        slot: Option<String>,
        intercept: Option<Intercept>,
        depth: usize,
    ) {
        if depth > 64 {
            return;
        }
        let idx = self.dirs.len();
        self.dirs.push(DirInfo {
            rel: rel.to_string(),
            parent,
            slot: slot.clone(),
            ..DirInfo::default()
        });
        let entries = list_dir(&root.join(rel));
        for (name, is_dir) in &entries {
            if *is_dir {
                continue;
            }
            let file = rel_join(rel, name);
            if let Some(sp) = special_name(name, &self.page_exts) {
                self.dirs[idx].files.insert(sp.to_string(), file);
            } else if let Some(url) = metadata_url(name, &self.page_exts) {
                self.dirs[idx].metadata.push(file.clone());
                let mut s = segs.clone();
                s.push(Seg::Static(url));
                if intercept.is_none() && slot.is_none() {
                    self.routes.push(Route {
                        id: format!("metadata:{file}"),
                        router: Router::App,
                        kind: RouteKind::Handler,
                        file,
                        dir: rel.to_string(),
                        pattern: pattern_string(&s),
                        segs: s,
                        slot: None,
                        intercept: None,
                        metadata: true,
                    });
                }
            }
        }
        for (name, is_dir) in &entries {
            if !*is_dir || crate::fsx::skip_dir(name) {
                continue;
            }
            let child = rel_join(rel, name);
            match parse_dir_name(name) {
                DirKind::Private => {}
                DirKind::Slot => {
                    let ci = self.dirs.len();
                    self.dirs[idx].slots.push(ci);
                    self.walk_app(
                        root,
                        &child,
                        Some(idx),
                        segs.clone(),
                        Some(child.clone()),
                        intercept.clone(),
                        depth + 1,
                    );
                }
                DirKind::Group => self.walk_app(
                    root,
                    &child,
                    Some(idx),
                    segs.clone(),
                    slot.clone(),
                    intercept.clone(),
                    depth + 1,
                ),
                DirKind::Intercept(marker, rest) => {
                    let mut s = match marker {
                        InterceptMarker::Same => segs.clone(),
                        InterceptMarker::Up(n) => {
                            let mut s = segs.clone();
                            for _ in 0..n {
                                s.pop();
                            }
                            s
                        }
                        InterceptMarker::Root => Vec::new(),
                    };
                    if let DirKind::Url(seg) = parse_dir_name(&rest) {
                        s.push(seg);
                    }
                    self.walk_app(
                        root,
                        &child,
                        Some(idx),
                        s,
                        slot.clone(),
                        Some(Intercept {
                            marker,
                            at: rel.to_string(),
                        }),
                        depth + 1,
                    );
                }
                DirKind::Url(seg) => {
                    let mut s = segs.clone();
                    s.push(seg);
                    self.walk_app(
                        root,
                        &child,
                        Some(idx),
                        s,
                        slot.clone(),
                        intercept.clone(),
                        depth + 1,
                    );
                }
            }
        }
        for (special, kind) in [("page", RouteKind::Page), ("route", RouteKind::Handler)] {
            if let Some(f) = self.dirs[idx].files.get(special).cloned() {
                self.routes.push(Route {
                    id: format!("{special}:{f}"),
                    router: Router::App,
                    kind,
                    file: f,
                    dir: rel.to_string(),
                    pattern: pattern_string(&segs),
                    segs: segs.clone(),
                    slot: slot.clone(),
                    intercept: intercept.clone(),
                    metadata: false,
                });
            }
        }
    }

    fn walk_pages(&mut self, root: &Path, top: &str, rel: &str, segs: Vec<Seg>, depth: usize) {
        if depth > 64 {
            return;
        }
        for (name, is_dir) in list_dir(&root.join(rel)) {
            let child = rel_join(rel, &name);
            if is_dir {
                if crate::fsx::skip_dir(&name) {
                    continue;
                }
                let mut s = segs.clone();
                if let DirKind::Url(seg) = parse_dir_name(&name) {
                    s.push(seg);
                } else {
                    s.push(Seg::Static(name.clone()));
                }
                self.walk_pages(root, top, &child, s, depth + 1);
                continue;
            }
            if name.contains(".d.") {
                continue;
            }
            let Some(base) = self.page_exts.iter().find_map(|e| {
                name.strip_suffix(e.as_str())
                    .and_then(|b| b.strip_suffix('.'))
            }) else {
                continue;
            };
            if rel == top && PAGES_SPECIAL.contains(&base) {
                self.pages_special.push(child);
                continue;
            }
            let mut s = segs.clone();
            if base != "index" {
                match parse_dir_name(base) {
                    DirKind::Url(seg) => s.push(seg),
                    _ => s.push(Seg::Static(base.to_string())),
                }
            }
            let is_api = matches!(segs.first(), Some(Seg::Static(x)) if x == "api");
            self.routes.push(Route {
                id: format!("{}:{child}", if is_api { "pages-api" } else { "pages" }),
                router: Router::Pages,
                kind: if is_api {
                    RouteKind::Handler
                } else {
                    RouteKind::Page
                },
                file: child,
                dir: rel.to_string(),
                pattern: pattern_string(&s),
                segs: s,
                slot: None,
                intercept: None,
                metadata: false,
            });
        }
    }

    fn find_conflicts(&mut self) {
        let mut by: BTreeMap<&str, (Vec<&str>, Vec<&str>)> = BTreeMap::new();
        for r in &self.routes {
            if r.slot.is_some() || r.intercept.is_some() || r.metadata {
                continue;
            }
            let e = by.entry(r.pattern.as_str()).or_default();
            match r.router {
                Router::App => e.0.push(&r.id),
                Router::Pages => e.1.push(&r.id),
            }
        }
        self.conflicts = by
            .into_iter()
            .filter(|(_, (a, p))| !a.is_empty() && !p.is_empty())
            .map(|(k, (a, p))| Conflict {
                pattern: k.to_string(),
                routes: a.iter().chain(&p).map(|s| s.to_string()).collect(),
            })
            .collect();
    }

    /// The route with this id.
    pub fn route(&self, id: &str) -> Option<&Route> {
        self.routes.iter().find(|r| r.id == id)
    }

    fn dir_index(&self, rel: &str) -> Option<usize> {
        self.dirs.iter().position(|d| d.rel == rel)
    }

    /// Root layouts: the first `layout` file on the chain of some page.
    pub fn root_layouts(&self) -> Vec<String> {
        let mut out = Vec::new();
        for r in &self.routes {
            if r.router != Router::App || r.kind != RouteKind::Page {
                continue;
            }
            let chain = self.chain(&r.dir);
            if let Some(l) = chain.iter().find_map(|&d| self.dirs[d].files.get("layout"))
                && !out.contains(l)
            {
                out.push(l.clone());
            }
        }
        out.sort();
        out
    }

    /// Directory indexes from the app dir down to `dir`.
    fn chain(&self, dir: &str) -> Vec<usize> {
        let mut v = Vec::new();
        let mut cur = self.dir_index(dir);
        while let Some(i) = cur {
            v.push(i);
            cur = self.dirs[i].parent;
        }
        v.reverse();
        v
    }

    /// The files Next composes for `route`.
    pub fn segment_closure(&self, route: &Route) -> SegmentClosure {
        let mut files = std::collections::BTreeSet::new();
        let mut soft = std::collections::BTreeSet::new();
        self.closure_into(route, &mut files, &mut soft, 0);
        let soft: Vec<String> = soft.into_iter().filter(|f| !files.contains(f)).collect();
        SegmentClosure {
            files: files.into_iter().collect(),
            soft,
        }
    }

    fn closure_into(
        &self,
        route: &Route,
        files: &mut std::collections::BTreeSet<String>,
        soft: &mut std::collections::BTreeSet<String>,
        depth: u32,
    ) {
        files.insert(route.file.clone());
        if depth > 8 {
            return;
        }
        if route.router == Router::Pages {
            if route.kind == RouteKind::Page {
                for f in &self.pages_special {
                    let base = f.rsplit('/').next().unwrap_or("");
                    if base.starts_with("_app.") || base.starts_with("_document.") {
                        files.insert(f.clone());
                    }
                }
            }
            return;
        }
        if route.kind == RouteKind::Handler {
            return;
        }
        let chain = self.chain(&route.dir);
        for &di in &chain {
            let d = &self.dirs[di];
            for b in BOUNDARY {
                if let Some(f) = d.files.get(*b) {
                    files.insert(f.clone());
                }
            }
            for m in &d.metadata {
                files.insert(m.clone());
            }
            if d.parent.is_none() {
                for g in ["global-error", "global-not-found"] {
                    if let Some(f) = d.files.get(g) {
                        files.insert(f.clone());
                    }
                }
            }
            if d.rel != route.dir
                && let Some(f) = d.files.get("default")
            {
                files.insert(f.clone());
            }
            for &si in &d.slots {
                let sd = &self.dirs[si];
                if route.slot.as_deref() == Some(sd.rel.as_str()) {
                    continue;
                }
                match self.match_slot_page(&sd.rel, route) {
                    Some(m) => self.closure_into(m, files, soft, depth + 1),
                    None => {
                        if let Some(f) = sd.files.get("default") {
                            files.insert(f.clone());
                        }
                        for b in BOUNDARY {
                            if let Some(f) = sd.files.get(*b) {
                                files.insert(f.clone());
                            }
                        }
                    }
                }
                for r in &self.routes {
                    if r.kind == RouteKind::Page
                        && r.intercept.is_some()
                        && r.slot.as_deref() == Some(sd.rel.as_str())
                    {
                        let mut s2 = std::collections::BTreeSet::new();
                        let mut plain = r.clone();
                        plain.intercept = None;
                        self.closure_into(&plain, &mut s2, soft, depth + 1);
                        soft.extend(s2);
                    }
                }
            }
        }
    }

    /// The page a slot renders for `route` on hard navigation: the slot's best
    /// match for the route's URL (its dynamic segments standing for unknown
    /// values).
    fn match_slot_page(&self, slot: &str, route: &Route) -> Option<&Route> {
        let cands: Vec<&Route> = self
            .routes
            .iter()
            .filter(|r| {
                r.kind == RouteKind::Page
                    && r.slot.as_deref() == Some(slot)
                    && r.intercept.is_none()
            })
            .collect();
        if let Some(e) = cands.iter().find(|r| r.pattern == route.pattern) {
            return Some(e);
        }
        let parts: Vec<Option<&str>> = route
            .segs
            .iter()
            .map(|s| match s {
                Seg::Static(v) => Some(v.as_str()),
                _ => None,
            })
            .collect();
        let mut best: Option<(&Route, Vec<u8>)> = None;
        for c in cands {
            let Some(rank) = rank_against(&c.segs, &parts) else {
                continue;
            };
            if best.as_ref().is_none_or(|(_, b)| rank < *b) {
                best = Some((c, rank));
            }
        }
        best.map(|(r, _)| r)
    }

    /// Absolute path of a table-relative file.
    pub fn abs(root: &Path, rel: &str) -> PathBuf {
        root.join(rel)
    }
}

/// Matches route segments against concrete parts (`None` = an unknown value
/// that only a dynamic segment takes). Returns the rank vector.
pub(crate) fn rank_against(segs: &[Seg], parts: &[Option<&str>]) -> Option<Vec<u8>> {
    let mut rank = Vec::with_capacity(segs.len());
    let mut i = 0;
    for s in segs {
        match s {
            Seg::Static(v) => {
                if parts.get(i).copied().flatten() != Some(v.as_str()) {
                    return None;
                }
                i += 1;
            }
            Seg::Dynamic(_) => {
                if i >= parts.len() {
                    return None;
                }
                i += 1;
            }
            Seg::CatchAll(_) => {
                if i >= parts.len() {
                    return None;
                }
                i = parts.len();
            }
            Seg::OptCatchAll(_) => i = parts.len(),
        }
        rank.push(s.rank());
    }
    (i == parts.len()).then_some(rank)
}
