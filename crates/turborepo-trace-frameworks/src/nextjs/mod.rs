//! The Next.js route adapter: which routes an app has, which
//! files each route renders, which inputs affect every route, and which URLs
//! route code requests at runtime.
//!
//! Next.js is not a test runner, so it does not implement [`crate::Framework`].
//! It contributes:
//!
//! - [`NextApp::load`]: route table ([`routes`]), `next.config` ([`config`]),
//!   microfrontends and middleware ([`mfe`]), Tailwind ([`tailwind`]);
//! - [`NextApp::matcher`]: URL / URL pattern → routes with Next's precedence
//!   ([`matcher`]);
//! - [`NextApp::framework_inputs`]: the global inputs as a [`FrameworkInputs`]
//!   (config files, JS entries, probes, memberships), so the store can record
//!   them like any other adapter's;
//! - [`fetch`]: static discovery of runtime URLs (`fetch`, SWR, axios,
//!   `redirect()`, ...);
//! - [`select`]: [`select::select_e2e`], route-level selection of Playwright
//!   specs over a [`turborepo_trace_graph::GraphView`].

pub mod build;
pub mod config;
pub mod fetch;
pub mod invoke;
pub mod jseval;
pub mod matcher;
pub mod mfe;
pub mod overlay;
pub mod p2r;
pub mod routes;
pub mod select;
pub mod source;
pub mod tailwind;
pub mod urlpat;

use std::path::{Path, PathBuf};

use config::NextConfig;
use matcher::Matcher;
use mfe::{Microfrontends, Middleware};
use routes::RouteTable;
use serde::Serialize;
use tailwind::TailwindInfo;

use crate::{
    fsx::PathFsx,
    inputs::{FrameworkInputs, GlobalInputs, MembershipSpec, Project, TestDiscovery},
};

/// Why a file is a global input (it affects every route).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GlobalWhy {
    /// `next.config.*`.
    NextConfig,
    /// A local module the config loads.
    NextConfigHelper,
    /// `middleware.ts` / `proxy.ts`.
    Middleware,
    /// `instrumentation.ts`, `instrumentation-client.ts`.
    Instrumentation,
    /// The first layout of some route (and what it imports, through the graph).
    RootLayout,
    /// `global-error`, `global-not-found`, and the root `not-found` / `error`.
    RootBoundary,
    /// `pages/_app`, `pages/_document`, `pages/_error`, `pages/404`,
    /// `pages/500`.
    PagesSpecial,
    /// A Tailwind entry stylesheet.
    GlobalCss,
    /// `postcss.config.*`.
    PostCss,
    /// `tailwind.config.*`.
    TailwindConfig,
    /// `microfrontends.json(c)`.
    Microfrontends,
    /// `vercel.json` / `vercel.ts`.
    VercelConfig,
    /// The app's `package.json`.
    PackageJson,
    /// `tsconfig.json` / `jsconfig.json`.
    Tsconfig,
}

/// One global input file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GlobalInput {
    /// The file (absolute).
    pub file: PathBuf,
    /// Why.
    pub why: GlobalWhy,
    /// Code: trace it as an entry (its imports are global too).
    pub code: bool,
    /// Also a route-matching input (config, middleware, microfrontends).
    pub routing: bool,
}

/// Inputs that affect every route.
#[derive(Clone, Debug, Default, Serialize)]
pub struct GlobalSet {
    /// Files.
    pub files: Vec<GlobalInput>,
    /// Missing paths whose creation affects every route (a middleware,
    /// instrumentation).
    pub probes: Vec<PathBuf>,
    /// Directory memberships: `public/**` (assets served by URL) and the env
    /// files (`.env*`: probed and listed, never read).
    pub memberships: Vec<MembershipSpec>,
}

/// Options for [`NextApp::load_with`].
#[derive(Clone, Debug, Default)]
pub struct LoadOptions {
    /// Use the evaluated rules of a build (`.next/routes-manifest.json`)
    /// instead of the config's.
    pub routes_manifest: Option<PathBuf>,
    /// Extra page extensions (united with the config's).
    pub page_extensions: Vec<String>,
}

/// A loaded Next.js app.
#[derive(Clone, Debug, Serialize)]
pub struct NextApp {
    /// App root (the directory of `next.config.*` / `package.json`).
    pub root: PathBuf,
    /// `package.json` name.
    pub package_name: Option<String>,
    /// The config.
    pub config: NextConfig,
    /// Routes.
    pub table: RouteTable,
    /// Microfrontends.
    pub mfe: Microfrontends,
    /// Middleware.
    pub middleware: Middleware,
    /// Tailwind.
    pub tailwind: TailwindInfo,
    /// Global inputs.
    pub globals: GlobalSet,
}

/// Loads the apps at `roots`, side by side, in the order of `roots`.
pub fn load_apps(roots: &[PathBuf]) -> Vec<NextApp> {
    use rayon::prelude::*;
    roots.par_iter().map(|r| NextApp::load(r)).collect()
}

const CODE_EXTS: &[&str] = &["ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts"];

fn with_exts(root: &Path, base: &str) -> Vec<PathBuf> {
    CODE_EXTS
        .iter()
        .map(|e| root.join(format!("{base}.{e}")))
        .collect()
}

/// Whether `dir` looks like a Next app root.
pub fn is_next_app(dir: &Path) -> bool {
    if config::find_config(dir).is_some() {
        return true;
    }
    let (a, p) = routes::router_dirs(dir);
    if a.is_none() && p.is_none() {
        return false;
    }
    crate::fsx::read_to_string(dir.join("package.json")).is_ok_and(|s| s.contains("\"next\""))
}

/// Every Next app root under `root`.
pub fn detect(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    crate::fsx::walk_dirs(root, &mut |d| {
        if d.join("package.json").fx_is_file() && is_next_app(d) {
            out.push(d.to_path_buf());
        }
    });
    out
}

impl NextApp {
    /// Loads the app at `root`.
    pub fn load(root: &Path) -> NextApp {
        NextApp::load_with(root, &LoadOptions::default())
    }

    /// Loads with options.
    pub fn load_with(root: &Path, opts: &LoadOptions) -> NextApp {
        let root = root.to_path_buf();
        let package_name = crate::fsx::read_to_string(root.join("package.json"))
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| v.get("name").and_then(|n| n.as_str()).map(String::from));
        let mut config = NextConfig::load(&root);
        if let Some(m) = &opts.routes_manifest
            && let Err(e) = config.use_routes_manifest(m)
        {
            config
                .uncertainties
                .push(crate::inputs::FrameworkUncertainty {
                    kind: turborepo_trace_core::UncertaintyKind::ParseIncomplete,
                    file: m.clone(),
                    line: 0,
                    field: None,
                    detail: format!("routes manifest unreadable: {e}"),
                });
        }
        let mut exts = config.page_extensions.clone();
        for e in &opts.page_extensions {
            if !exts.contains(e) {
                exts.push(e.clone());
            }
        }
        let table = RouteTable::scan(&root, &exts);
        let mfe = Microfrontends::load(&root, package_name.as_deref());
        let middleware = Middleware::load(&root);
        let mut css = Vec::new();
        for d in [
            table.app_dir.as_deref(),
            Some("styles"),
            Some("src/styles"),
            Some("src"),
        ]
        .into_iter()
        .flatten()
        {
            crate::fsx::walk_files(&root.join(d), &mut |p| {
                if p.extension().is_some_and(|e| e == "css") {
                    css.push(p.to_path_buf());
                }
            });
        }
        css.sort();
        css.dedup();
        let tailwind = TailwindInfo::detect(&root, &css);
        let mut app = NextApp {
            root,
            package_name,
            config,
            table,
            mfe,
            middleware,
            tailwind,
            globals: GlobalSet::default(),
        };
        app.globals = app.compute_globals();
        app
    }

    fn compute_globals(&self) -> GlobalSet {
        let mut g = GlobalSet::default();
        let root = &self.root;
        let mut add = |file: PathBuf, why: GlobalWhy, code: bool, routing: bool| {
            if !g.files.iter().any(|x| x.file == file) {
                g.files.push(GlobalInput {
                    file,
                    why,
                    code,
                    routing,
                });
            }
        };
        if let Some(f) = &self.config.file {
            add(f.clone(), GlobalWhy::NextConfig, true, true);
        }
        for f in &self.config.files {
            let code = self.config.code_files.contains(f);
            add(f.clone(), GlobalWhy::NextConfigHelper, code, true);
        }
        for f in &self.middleware.files {
            add(f.clone(), GlobalWhy::Middleware, true, true);
        }
        if let Some(f) = &self.mfe.file {
            add(f.clone(), GlobalWhy::Microfrontends, false, true);
        }
        for base in [
            "instrumentation",
            "instrumentation-client",
            "src/instrumentation",
            "src/instrumentation-client",
        ] {
            for p in with_exts(root, base) {
                if p.fx_is_file() {
                    add(p, GlobalWhy::Instrumentation, true, false);
                }
            }
        }
        for l in self.table.root_layouts() {
            add(root.join(l), GlobalWhy::RootLayout, true, false);
        }
        if let Some(d) = self.table.dirs.first() {
            for k in ["global-error", "global-not-found", "not-found", "error"] {
                if let Some(f) = d.files.get(k) {
                    add(root.join(f), GlobalWhy::RootBoundary, true, false);
                }
            }
        }
        for f in &self.table.pages_special {
            add(root.join(f), GlobalWhy::PagesSpecial, true, false);
        }
        for f in &self.tailwind.css_files {
            add(f.clone(), GlobalWhy::GlobalCss, false, false);
        }
        for f in &self.tailwind.config_files {
            add(f.clone(), GlobalWhy::TailwindConfig, true, false);
        }
        for p in with_exts(root, "postcss.config") {
            if p.fx_is_file() {
                add(p, GlobalWhy::PostCss, true, false);
            }
        }
        for (n, why, code) in [
            ("vercel.json", GlobalWhy::VercelConfig, false),
            ("vercel.ts", GlobalWhy::VercelConfig, true),
            ("package.json", GlobalWhy::PackageJson, false),
            ("tsconfig.json", GlobalWhy::Tsconfig, false),
            ("jsconfig.json", GlobalWhy::Tsconfig, false),
        ] {
            let p = root.join(n);
            if p.fx_is_file() {
                add(p, why, code, false);
            }
        }
        // Probes: files whose creation changes every route.
        let mut probes: Vec<PathBuf> = mfe::MIDDLEWARE_NAMES.iter().map(|n| root.join(n)).collect();
        for base in ["instrumentation", "instrumentation-client"] {
            probes.extend(with_exts(root, base));
        }
        probes.extend(mfe::MFE_NAMES.iter().map(|n| root.join(n)));
        probes.extend(config::CONFIG_NAMES.iter().map(|n| root.join(n)));
        probes.retain(|p| !p.fx_exists());
        g.probes = probes;
        g.memberships.push(MembershipSpec {
            dir: root.join("public"),
            pattern: "**".into(),
            reason: "public/ assets are served by URL".into(),
        });
        g.memberships.push(MembershipSpec {
            dir: root.clone(),
            pattern: ".env*".into(),
            reason: "env files (listed, never read)".into(),
        });
        g
    }

    /// The memberships that make up the route table: special files under `app/`
    /// and `pages/` (adding or removing one can change which route a URL
    /// matches).
    pub fn route_table_memberships(&self) -> Vec<MembershipSpec> {
        let exts = self.table.page_exts.join(",");
        let mut out = Vec::new();
        if let Some(a) = &self.table.app_dir {
            out.push(MembershipSpec {
                dir: self.root.join(a),
                pattern: format!("**/{{{}}}.{{{exts}}}", routes::SPECIAL.join(",")),
                reason: "App Router special files (the route table)".into(),
            });
        }
        if let Some(p) = &self.table.pages_dir {
            out.push(MembershipSpec {
                dir: self.root.join(p),
                pattern: format!("**/*.{{{exts}}}"),
                reason: "Pages Router files (the route table)".into(),
            });
        }
        out
    }

    /// A matcher over this app.
    pub fn matcher(&self) -> Matcher<'_> {
        let public = self.root.join("public");
        Matcher::new(&self.table, &self.config, &self.mfe, &self.middleware, None)
            .with_public(public)
    }

    /// The adapter's contribution in the shared shape: one project whose global
    /// inputs are the app's global set and route-table memberships.
    pub fn framework_inputs(&self) -> FrameworkInputs {
        let mut global = GlobalInputs::default();
        for f in &self.globals.files {
            global.config_files.push(f.file.clone());
            if f.code {
                global.js_entries.push(f.file.clone());
            }
        }
        global.probes = self.globals.probes.clone();
        global.memberships = self.globals.memberships.clone();
        global.memberships.extend(self.route_table_memberships());
        let mut uncertainties = self.config.uncertainties.clone();
        for e in &self.mfe.errors {
            uncertainties.push(crate::inputs::FrameworkUncertainty {
                kind: turborepo_trace_core::UncertaintyKind::UnsupportedSyntax,
                file: self.mfe.file.clone().unwrap_or_default(),
                line: 0,
                field: None,
                detail: e.clone(),
            });
        }
        FrameworkInputs {
            framework: "nextjs",
            config_file: self.config.file.clone(),
            projects: vec![Project {
                name: self
                    .package_name
                    .clone()
                    .unwrap_or_else(|| self.root.to_string_lossy().into_owned()),
                config_file: self.config.file.clone(),
                root_dir: self.root.clone(),
                global,
                discovery: TestDiscovery::default(),
                uncertainties: Vec::new(),
                ..Project::default()
            }],
            uncertainties,
        }
    }
}
