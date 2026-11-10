//! Reads a Next.js build output directory (`.next`): the route entries and, for
//! each, the output files the build itself says the route loads.
//!
//! | File (relative to the dist dir) | Gives |
//! |---|---|
//! | `BUILD_ID` | the build id (embedded in many outputs) |
//! | `server/app-paths-manifest.json`, `app-path-routes-manifest.json` | App Router entries and their pathnames |
//! | `server/pages-manifest.json` | Pages Router entries |
//! | `<entry>.nft.json` | the server files an entry needs (chunks inside the dist dir, traced files outside it) |
//! | `<entry>_client-reference-manifest.js` | client chunks and CSS of an App Router entry |
//! | `build-manifest.json`, `<entry dir>/build-manifest.json`, `app-build-manifest.json` | bootstrap and per-page client files |
//! | `<entry dir>/react-loadable-manifest.json` | lazily loaded client chunks |
//! | `server/server-reference-manifest.json` | server action ids (salted per build) with file and export |
//! | `server/middleware-manifest.json`, `server/functions-config-manifest.json` | middleware and edge entries with their files and matchers |
//! | `prerender-manifest.json` | prerendered paths and the route each belongs to |
//! | `routes-manifest.json` | evaluated redirects, rewrites, headers; static and dynamic route regexes |
//! | `required-server-files.json` | the evaluated config |
//!
//! Nothing here interprets chunk contents; that is [`super::fingerprint`].

use std::{
    collections::BTreeSet,
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::fsx;

/// Which bundler produced the build.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Bundler {
    /// Turbopack (the default from Next.js 16).
    Turbopack,
    /// webpack.
    Webpack,
}

/// The router an entry belongs to.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildRouter {
    /// `app/`.
    App,
    /// `pages/`.
    Pages,
}

/// Page or handler.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildRouteKind {
    /// Renders a document.
    Page,
    /// `route.ts` or `pages/api/*`.
    Handler,
}

/// The runtime an entry runs in.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Runtime {
    /// Node.js.
    Node,
    /// Edge.
    Edge,
}

/// A file a trace lists outside the dist dir.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ExternalFile {
    /// Path relative to the project dir (may start with `..`).
    pub path: String,
    /// The content hash the trace carries (Turbopack), if any.
    pub hash: Option<String>,
}

/// One `.nft.json` trace.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trace {
    /// Traced files inside the dist dir (dist-relative).
    pub inside: Vec<String>,
    /// Traced files outside it (`node_modules`, files read through `fs`).
    pub external: Vec<ExternalFile>,
    /// Whether the trace file existed.
    pub present: bool,
}

/// One route entry of the build.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildRoute {
    /// The manifest key (`/about/page`, `/api/x/route`, `/legacy`).
    pub key: String,
    /// The URL pattern (`/about`, `/blog/[slug]`).
    pub pathname: String,
    /// Router.
    pub router: BuildRouter,
    /// Page or handler.
    pub kind: BuildRouteKind,
    /// Runtime.
    pub runtime: Runtime,
    /// The server entry file (dist-relative).
    pub entry: String,
    /// Its trace.
    pub trace: Trace,
    /// The client reference manifest file, if any (dist-relative).
    pub client_manifest: Option<String>,
    /// Client JS and CSS files the manifests list for this entry
    /// (dist-relative, deduplicated).
    pub client_files: Vec<String>,
    /// Files of an edge entry (`middleware-manifest.json` `functions`).
    pub edge_files: Vec<String>,
}

/// A redirect, rewrite or header rule as the build evaluated it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildRule {
    /// `redirect`, `header`, `rewrite:beforeFiles`, `rewrite:afterFiles`,
    /// `rewrite:fallback`.
    pub phase: String,
    /// The source pattern.
    pub source: String,
    /// The compiled regex (JavaScript syntax).
    pub regex: String,
    /// Destination (redirects, rewrites).
    pub destination: Option<String>,
    /// A rule Next adds itself (trailing slash, basePath).
    pub internal: bool,
    /// The whole rule, for hashing and conditions (`has`, `missing`, `headers`,
    /// `statusCode`).
    pub raw: Value,
}

/// A route regex of the routes manifest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteRegex {
    /// The route (`/blog/[slug]`).
    pub page: String,
    /// The regex (JavaScript syntax).
    pub regex: String,
}

/// `routes-manifest.json`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingManifest {
    /// `basePath`.
    pub base_path: String,
    /// Redirects, headers and rewrites in evaluation order per phase.
    pub rules: Vec<BuildRule>,
    /// Static routes.
    pub static_routes: Vec<RouteRegex>,
    /// Dynamic routes, in Next's match order.
    pub dynamic_routes: Vec<RouteRegex>,
    /// `i18n`, `caseSensitive` and other switches that change matching.
    pub switches: Value,
}

/// A server action.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerAction {
    /// The id (salted per build).
    pub id: String,
    /// `node` or `edge`.
    pub layer: String,
    /// The source file (project-relative).
    pub filename: String,
    /// The exported name.
    pub exported_name: String,
    /// The entries that bundle it (`app/dashboard/settings/page`).
    pub workers: Vec<String>,
}

impl ServerAction {
    /// The stable identity: `file#export`.
    pub fn stable_id(&self) -> String {
        format!("{}#{}", self.filename, self.exported_name)
    }
}

/// A middleware or instrumentation entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalEntry {
    /// `middleware`, `instrumentation`.
    pub name: String,
    /// Its files (dist-relative): entry, manifest-listed files, traced files
    /// inside the dist.
    pub files: Vec<String>,
    /// Traced files outside the dist.
    pub external: Vec<ExternalFile>,
    /// Matchers (middleware): the compiled regexes with their sources.
    pub matchers: Value,
}

/// A prerendered path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prerender {
    /// The concrete path (`/blog/first`).
    pub path: String,
    /// The route it was rendered from (`/blog/[slug]`).
    pub src_route: String,
}

/// A loaded build output.
#[derive(Clone, Debug)]
pub struct NextBuild {
    /// The dist dir (`.next`).
    pub dist: PathBuf,
    /// The project dir (where `next build` ran).
    pub project: PathBuf,
    /// The build id.
    pub build_id: String,
    /// The bundler.
    pub bundler: Bundler,
    /// Every output file considered (dist-relative, `/`-separated):
    /// `server/**`, `static/**` and the top-level manifests, without source
    /// maps, caches, traces and type stubs.
    pub files: BTreeSet<String>,
    /// Route entries, sorted by pathname then key.
    pub routes: Vec<BuildRoute>,
    /// Bootstrap client files every App Router page loads.
    pub app_bootstrap: Vec<String>,
    /// Bootstrap client files every Pages Router page loads (`/_app`, main,
    /// polyfills).
    pub pages_bootstrap: Vec<String>,
    /// Prerendered paths.
    pub prerenders: Vec<Prerender>,
    /// The routes manifest.
    pub routing: RoutingManifest,
    /// Server actions.
    pub actions: Vec<ServerAction>,
    /// Middleware and instrumentation.
    pub globals: Vec<GlobalEntry>,
    /// The evaluated config (`required-server-files.json` `config`).
    pub config: Value,
    /// `images-manifest.json`.
    pub images: Value,
    /// Manifests that were missing or unreadable (the build may be older or
    /// partial).
    pub warnings: Vec<String>,
}

/// Top-level directories of the dist dir that are not build output of routes.
const SKIP_TOP: &[&str] = &[
    "cache",
    "types",
    "trace",
    "trace-build",
    "dev",
    "diagnostics",
    "build",
    "turbopack",
    "standalone",
];

/// Pages Router entries that are not routes of their own.
pub const PAGES_INTERNAL: &[&str] = &["/_app", "/_document", "/_error"];

fn rel_str(p: &Path) -> String {
    p.components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            Component::ParentDir => Some("..".to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn walk(dir: &Path, rel: &str, depth: usize, out: &mut BTreeSet<String>) {
    if depth > 64 {
        return;
    }
    let Some(entries) = fsx::read_dir(dir) else {
        return;
    };
    for (p, is_dir) in entries {
        let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let r = if rel.is_empty() {
            name.to_string()
        } else {
            format!("{rel}/{name}")
        };
        if is_dir {
            if rel.is_empty() && SKIP_TOP.contains(&name) {
                continue;
            }
            walk(&p, &r, depth + 1, out);
        } else if !name.ends_with(".map") && !(rel.is_empty() && SKIP_TOP.contains(&name)) {
            out.insert(r);
        }
    }
}

/// Percent-decodes `%XX` sequences (manifest paths of route groups and dynamic
/// segments).
pub(crate) fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            out.push(h * 16 + l);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// A manifest path of a static file as a dist-relative path:
/// `/_next/static/x.js`, `static/x.js` and (webpack client modules) `x.js`
/// under `static/chunks`.
fn static_path(s: &str) -> String {
    let s = s.strip_prefix('/').unwrap_or(s);
    let s = s.strip_prefix("_next/").unwrap_or(s);
    percent_decode(s)
}

/// The JSON value assigned in a `globalThis.X["key"] = {...};` manifest script.
pub(crate) fn assigned_json(src: &str) -> Option<Value> {
    // The last assignment is the manifest; an earlier one initializes the
    // container.
    let at = src
        .rfind("] = ")
        .map(|i| i + 4)
        .or_else(|| src.rfind("]=").map(|i| i + 2))?;
    let rest = src[at..].trim_start();
    if !rest.starts_with('{') {
        return None;
    }
    serde_json::Deserializer::from_str(rest)
        .into_iter::<Value>()
        .next()?
        .ok()
}

fn strs(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

impl NextBuild {
    /// Loads the build in `<project>/.next`.
    pub fn load(project: &Path) -> Result<NextBuild, String> {
        NextBuild::load_dist(project, &project.join(".next"))
    }

    /// Loads the build output in `dist`, built from `project`.
    pub fn load_dist(project: &Path, dist: &Path) -> Result<NextBuild, String> {
        let project = fsx::normalize(project);
        let dist = fsx::normalize(dist);
        let build_id = fsx::read_to_string(dist.join("BUILD_ID"))
            .map_err(|e| {
                format!(
                    "{}: no BUILD_ID ({e}); not a production build",
                    dist.display()
                )
            })?
            .trim()
            .to_string();
        let mut files = BTreeSet::new();
        walk(&dist, "", 0, &mut files);
        let mut b = NextBuild {
            dist,
            project,
            build_id,
            bundler: Bundler::Webpack,
            files,
            routes: Vec::new(),
            app_bootstrap: Vec::new(),
            pages_bootstrap: Vec::new(),
            prerenders: Vec::new(),
            routing: RoutingManifest::default(),
            actions: Vec::new(),
            globals: Vec::new(),
            config: Value::Null,
            images: Value::Null,
            warnings: Vec::new(),
        };
        if b.files.iter().any(|f| f.contains("[turbopack]_runtime")) {
            b.bundler = Bundler::Turbopack;
        }
        b.load_routes();
        b.load_prerenders();
        b.load_routing();
        b.load_actions();
        b.load_globals();
        b.config = b
            .json("required-server-files.json")
            .and_then(|mut v| v.get_mut("config").map(Value::take))
            .unwrap_or(Value::Null);
        b.images = b.json_opt("images-manifest.json").unwrap_or(Value::Null);
        Ok(b)
    }

    /// Reads a dist-relative file.
    pub fn read(&self, rel: &str) -> Option<Vec<u8>> {
        fsx::read(self.dist.join(rel)).ok()
    }

    fn json_opt(&self, rel: &str) -> Option<Value> {
        serde_json::from_slice(&self.read(rel)?).ok()
    }

    fn json(&mut self, rel: &str) -> Option<Value> {
        let v = self.json_opt(rel);
        if v.is_none() {
            self.warnings.push(format!("{rel}: missing or unreadable"));
        }
        v
    }

    /// Reads `<entry>.nft.json`.
    pub fn trace(&self, entry: &str) -> Trace {
        let rel = format!("{entry}.nft.json");
        let Some(v) = self.json_opt(&rel) else {
            return Trace::default();
        };
        let base = self.dist.join(entry);
        let base = base.parent().unwrap_or(&self.dist);
        let hashes = v.get("fileHashes").and_then(Value::as_array);
        let mut t = Trace {
            present: true,
            ..Trace::default()
        };
        for (i, f) in strs(v.get("files")).iter().enumerate() {
            let abs = fsx::join(base, f);
            if let Ok(r) = abs.strip_prefix(&self.dist) {
                t.inside.push(rel_str(r));
            } else {
                let path = match abs.strip_prefix(&self.project) {
                    Ok(r) => rel_str(r),
                    Err(_) => rel_from(&self.project, &abs),
                };
                let hash = hashes
                    .and_then(|h| h.get(i))
                    .and_then(Value::as_str)
                    .map(String::from);
                t.external.push(ExternalFile { path, hash });
            }
        }
        t.inside.sort();
        t.external.sort();
        t
    }

    fn load_routes(&mut self) {
        let root = self.json("build-manifest.json").unwrap_or(Value::Null);
        let root_main = strs(root.get("rootMainFiles"));
        let polyfills = strs(root.get("polyfillFiles"));
        self.app_bootstrap = root_main
            .iter()
            .chain(&polyfills)
            .map(|s| static_path(s))
            .collect();
        let pages_of = |k: &str| strs(root.get("pages").and_then(|p| p.get(k)));
        self.pages_bootstrap = pages_of("/_app")
            .iter()
            .chain(&polyfills)
            .map(|s| static_path(s))
            .collect();
        let edge = self
            .json_opt("server/middleware-manifest.json")
            .unwrap_or(Value::Null);
        let edge_files = |page: &str| -> Vec<String> {
            edge.get("functions")
                .and_then(Value::as_object)
                .into_iter()
                .flatten()
                .filter(|(_, f)| f.get("page").and_then(Value::as_str) == Some(page))
                .flat_map(|(_, f)| strs(f.get("files")))
                .collect()
        };
        let app_build = self.json_opt("app-build-manifest.json");

        // App Router.
        let app_paths = self.json_opt("server/app-paths-manifest.json");
        let pathnames = self
            .json_opt("app-path-routes-manifest.json")
            .unwrap_or(Value::Null);
        for (key, file) in app_paths
            .as_ref()
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            let Some(file) = file.as_str() else { continue };
            let entry = format!("server/{file}");
            let pathname = pathnames
                .get(key)
                .and_then(Value::as_str)
                .map_or_else(|| key.clone(), String::from);
            let kind = if key.ends_with("/route") {
                BuildRouteKind::Handler
            } else {
                BuildRouteKind::Page
            };
            let side = entry.strip_suffix(".js").unwrap_or(&entry).to_string();
            let mut client: Vec<String> = Vec::new();
            let cm = format!("{side}_client-reference-manifest.js");
            let client_manifest = self.files.contains(&cm).then_some(cm);
            if kind == BuildRouteKind::Page {
                if let Some(m) = client_manifest
                    .as_ref()
                    .and_then(|p| self.read(p))
                    .and_then(|b| assigned_json(&String::from_utf8_lossy(&b)))
                {
                    for module in m
                        .get("clientModules")
                        .and_then(Value::as_object)
                        .into_iter()
                        .flatten()
                    {
                        for c in strs(module.1.get("chunks")) {
                            if c.ends_with(".js") || c.ends_with(".css") {
                                // webpack lists chunk ids and bare names under static/chunks.
                                client.push(if c.contains("static/") {
                                    static_path(&c)
                                } else {
                                    format!("static/chunks/{}", percent_decode(&c))
                                });
                            }
                        }
                    }
                    for (_, l) in m
                        .get("entryJSFiles")
                        .and_then(Value::as_object)
                        .into_iter()
                        .flatten()
                    {
                        client.extend(strs(Some(l)).iter().map(|s| static_path(s)));
                    }
                    for (_, l) in m
                        .get("entryCSSFiles")
                        .and_then(Value::as_object)
                        .into_iter()
                        .flatten()
                    {
                        for c in l.as_array().into_iter().flatten() {
                            if let Some(s) = c.as_str().or_else(|| c.get("path")?.as_str()) {
                                client.push(static_path(s));
                            }
                        }
                    }
                } else if client_manifest.is_some() {
                    self.warnings
                        .push(format!("{side}_client-reference-manifest.js: not parsed"));
                }
                // Turbopack writes a build manifest per entry; webpack one for the app.
                match self.json_opt(&format!("{side}/build-manifest.json")) {
                    Some(bm) => {
                        for k in ["rootMainFiles", "polyfillFiles"] {
                            client.extend(strs(bm.get(k)).iter().map(|s| static_path(s)));
                        }
                        for (_, l) in bm
                            .get("pages")
                            .and_then(Value::as_object)
                            .into_iter()
                            .flatten()
                        {
                            client.extend(strs(Some(l)).iter().map(|s| static_path(s)));
                        }
                    }
                    None => client.extend(self.app_bootstrap.iter().cloned()),
                }
                if let Some(ab) = &app_build {
                    client.extend(
                        strs(ab.get("pages").and_then(|p| p.get(key)))
                            .iter()
                            .map(|s| static_path(s)),
                    );
                }
                if let Some(lm) = self.json_opt(&format!("{side}/react-loadable-manifest.json")) {
                    for (_, v) in lm.as_object().into_iter().flatten() {
                        client.extend(strs(v.get("files")).iter().map(|s| static_path(s)));
                    }
                }
            }
            client.sort();
            client.dedup();
            let ef = edge_files(key);
            let trace = self.trace(&entry);
            self.routes.push(BuildRoute {
                key: key.clone(),
                pathname,
                router: BuildRouter::App,
                kind,
                runtime: if ef.is_empty() {
                    Runtime::Node
                } else {
                    Runtime::Edge
                },
                entry,
                trace,
                client_manifest,
                client_files: client,
                edge_files: ef,
            });
        }

        // Pages Router.
        let pages = self.json_opt("server/pages-manifest.json");
        for (pathname, file) in pages
            .as_ref()
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            let Some(file) = file.as_str() else { continue };
            let entry = format!("server/{file}");
            let kind = if pathname.starts_with("/api/") || pathname == "/api" {
                BuildRouteKind::Handler
            } else {
                BuildRouteKind::Page
            };
            let mut client: Vec<String> = Vec::new();
            if kind == BuildRouteKind::Page {
                client.extend(pages_of(pathname).iter().map(|s| static_path(s)));
                client.extend(self.pages_bootstrap.iter().cloned());
                client.sort();
                client.dedup();
            }
            let ef = edge_files(pathname);
            let trace = self.trace(&entry);
            self.routes.push(BuildRoute {
                key: pathname.clone(),
                pathname: pathname.clone(),
                router: BuildRouter::Pages,
                kind,
                runtime: if ef.is_empty() {
                    Runtime::Node
                } else {
                    Runtime::Edge
                },
                entry,
                trace,
                client_manifest: None,
                client_files: client,
                edge_files: ef,
            });
        }
        if app_paths.is_none() && pages.is_none() {
            self.warnings
                .push("no app-paths-manifest.json or pages-manifest.json".into());
        }
        self.routes
            .sort_by(|a, b| (&a.pathname, &a.key).cmp(&(&b.pathname, &b.key)));
    }

    fn load_prerenders(&mut self) {
        let Some(v) = self.json("prerender-manifest.json") else {
            return;
        };
        for (path, r) in v
            .get("routes")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            let src = r
                .get("srcRoute")
                .and_then(Value::as_str)
                .unwrap_or(path)
                .to_string();
            self.prerenders.push(Prerender {
                path: path.clone(),
                src_route: src,
            });
        }
    }

    fn load_routing(&mut self) {
        let Some(v) = self.json("routes-manifest.json") else {
            return;
        };
        let mut rules = Vec::new();
        let mut add = |phase: &str, list: Option<&Value>| {
            for r in list.and_then(Value::as_array).into_iter().flatten() {
                let s = |k: &str| r.get(k).and_then(Value::as_str).map(String::from);
                rules.push(BuildRule {
                    phase: phase.to_string(),
                    source: s("source").unwrap_or_default(),
                    regex: s("regex").unwrap_or_default(),
                    destination: s("destination"),
                    internal: r.get("internal").and_then(Value::as_bool) == Some(true),
                    raw: r.clone(),
                });
            }
        };
        add("redirect", v.get("redirects"));
        add("header", v.get("headers"));
        match v.get("rewrites") {
            // Older builds: a plain list (all afterFiles).
            Some(l @ Value::Array(_)) => add("rewrite:afterFiles", Some(l)),
            Some(o) => {
                add("rewrite:beforeFiles", o.get("beforeFiles"));
                add("rewrite:afterFiles", o.get("afterFiles"));
                add("rewrite:fallback", o.get("fallback"));
            }
            None => {}
        }
        let regexes = |k: &str| -> Vec<RouteRegex> {
            v.get(k)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|r| {
                    Some(RouteRegex {
                        page: r.get("page")?.as_str()?.to_string(),
                        regex: r.get("regex")?.as_str()?.to_string(),
                    })
                })
                .collect()
        };
        let mut switches = serde_json::Map::new();
        for k in [
            "i18n",
            "caseSensitive",
            "skipProxyUrlNormalize",
            "rsc",
            "rewriteHeaders",
        ] {
            if let Some(x) = v.get(k) {
                switches.insert(k.to_string(), x.clone());
            }
        }
        self.routing = RoutingManifest {
            base_path: v
                .get("basePath")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            rules,
            static_routes: regexes("staticRoutes"),
            dynamic_routes: regexes("dynamicRoutes"),
            switches: Value::Object(switches),
        };
    }

    fn load_actions(&mut self) {
        let Some(v) = self.json_opt("server/server-reference-manifest.json") else {
            return;
        };
        for layer in ["node", "edge"] {
            for (id, a) in v
                .get(layer)
                .and_then(Value::as_object)
                .into_iter()
                .flatten()
            {
                let s = |k: &str| {
                    a.get(k)
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string()
                };
                let mut workers: Vec<String> = a
                    .get("workers")
                    .and_then(Value::as_object)
                    .map(|w| w.keys().cloned().collect())
                    .unwrap_or_default();
                workers.sort();
                self.actions.push(ServerAction {
                    id: id.clone(),
                    layer: layer.to_string(),
                    filename: s("filename"),
                    exported_name: s("exportedName"),
                    workers,
                });
            }
        }
        self.actions.sort_by(|a, b| a.id.cmp(&b.id));
    }

    fn load_globals(&mut self) {
        let edge = self
            .json_opt("server/middleware-manifest.json")
            .unwrap_or(Value::Null);
        let fcm = self
            .json_opt("server/functions-config-manifest.json")
            .unwrap_or(Value::Null);
        // Middleware: an edge entry of the middleware manifest, or a Node entry
        // (`server/middleware.js`) whose matchers are in the functions config.
        let mut files: Vec<String> = Vec::new();
        let mut matchers: Vec<Value> = Vec::new();
        for (_, m) in edge
            .get("middleware")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            files.extend(strs(m.get("files")));
            matchers.extend(
                m.get("matchers")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .cloned(),
            );
        }
        if let Some(m) = fcm.get("functions").and_then(|f| f.get("/_middleware")) {
            matchers.extend(
                m.get("matchers")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .cloned(),
            );
        }
        for (name, entry) in [
            ("middleware", "server/middleware.js"),
            ("instrumentation", "server/instrumentation.js"),
        ] {
            let mut fs: Vec<String> = if name == "middleware" {
                std::mem::take(&mut files)
            } else {
                Vec::new()
            };
            let mut external = Vec::new();
            if self.files.contains(entry) {
                fs.push(entry.to_string());
                let t = self.trace(entry);
                fs.extend(t.inside);
                external = t.external;
            }
            // Edge instrumentation and middleware chunks.
            let prefix = format!("server/edge-{name}");
            fs.extend(
                self.files
                    .iter()
                    .filter(|f| f.starts_with(&prefix))
                    .cloned(),
            );
            fs.sort();
            fs.dedup();
            if fs.is_empty() && (name != "middleware" || matchers.is_empty()) {
                continue;
            }
            self.globals.push(GlobalEntry {
                name: name.to_string(),
                files: fs,
                external,
                matchers: if name == "middleware" {
                    Value::Array(std::mem::take(&mut matchers))
                } else {
                    Value::Null
                },
            });
        }
    }

    /// The route entries that are routes of the app (no `/_app`, `/_document`,
    /// `/_error`).
    pub fn public_routes(&self) -> impl Iterator<Item = &BuildRoute> {
        self.routes.iter().filter(|r| {
            !(r.router == BuildRouter::Pages && PAGES_INTERNAL.contains(&r.key.as_str()))
        })
    }

    /// The output files of the prerenders of `route` (HTML, RSC payloads,
    /// segment payloads, metadata, Pages Router data).
    pub fn prerender_files(&self, route: &BuildRoute) -> Vec<String> {
        let dir = match route.router {
            BuildRouter::App => "server/app",
            BuildRouter::Pages => "server/pages",
        };
        let mut bases: Vec<String> = self
            .prerenders
            .iter()
            .filter(|p| p.src_route == route.pathname || p.path == route.pathname)
            .map(|p| p.path.clone())
            .collect();
        if route.router == BuildRouter::Pages {
            // Automatically static pages are not in the prerender manifest.
            bases.push(route.pathname.clone());
        }
        bases.sort();
        bases.dedup();
        let mut out = Vec::new();
        for p in bases {
            let base = if p == "/" {
                format!("{dir}/index")
            } else {
                format!("{dir}{p}")
            };
            for ext in [".html", ".rsc", ".meta", ".json", ".body", ".prefetch.rsc"] {
                let f = format!("{base}{ext}");
                if self.files.contains(&f) {
                    out.push(f);
                }
            }
            let seg = format!("{base}.segments/");
            out.extend(
                self.files
                    .range(seg.clone()..)
                    .take_while(|f| f.starts_with(&seg))
                    .cloned(),
            );
        }
        out.sort();
        out.dedup();
        out
    }
}

/// `to` relative to `from` (both absolute, normalized), with `..` segments.
fn rel_from(from: &Path, to: &Path) -> String {
    let a: Vec<_> = from.components().collect();
    let b: Vec<_> = to.components().collect();
    let common = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let mut parts: Vec<String> = vec!["..".to_string(); a.len() - common];
    parts.extend(
        b[common..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );
    parts.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assigned_json_reads_the_last_assignment() {
        let src = "globalThis.__RSC_MANIFEST = globalThis.__RSC_MANIFEST || \
                   {};\nglobalThis.__RSC_MANIFEST[\"/about/page\"] = \
                   {\"clientModules\":{\"a\":{\"chunks\":[\"/_next/static/chunks/x.js\"]}}};\n";
        let v = assigned_json(src).unwrap();
        assert_eq!(
            v["clientModules"]["a"]["chunks"][0],
            "/_next/static/chunks/x.js"
        );
        let compact = "globalThis.__RSC_MANIFEST=(globalThis.__RSC_MANIFEST||{});globalThis.\
                       __RSC_MANIFEST[\"/page\"]={\"entryJSFiles\":{}}";
        assert!(
            assigned_json(compact)
                .unwrap()
                .get("entryJSFiles")
                .is_some()
        );
        assert!(assigned_json("var x = 1").is_none());
    }

    #[test]
    fn static_paths() {
        assert_eq!(
            static_path("/_next/static/chunks/a.js"),
            "static/chunks/a.js"
        );
        assert_eq!(static_path("static/chunks/a.css"), "static/chunks/a.css");
        assert_eq!(
            static_path("/_next/static/chunks/app/%5Bslug%5D/page-1.js"),
            "static/chunks/app/[slug]/page-1.js"
        );
    }

    #[test]
    fn relative_paths() {
        assert_eq!(
            rel_from(
                Path::new("/r/apps/web"),
                Path::new("/r/node_modules/x/i.js")
            ),
            "../../node_modules/x/i.js"
        );
    }
}
