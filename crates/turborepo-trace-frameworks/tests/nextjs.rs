//! Next.js route model, matcher, fetch discovery, Playwright extraction and e2e
//! selection over the `fixtures/frameworks/next-*` apps.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use serde_json::json;
use turborepo_trace_frameworks::{
    Framework,
    common::SimpleResolver,
    framework,
    nextjs::{
        NextApp,
        fetch::default_url_apis,
        jseval::{Files, JsFile},
        matcher::RequestContext,
        select::{
            self, AmbiguityPolicy, Contents, Reason, RouteTableMode, SelectOptions, select_e2e,
        },
        tailwind::TailwindMode,
    },
    playwright::{
        self, Playwright,
        visits::{ExtractOptions, SpecVisits, extract_visits},
    },
};
use turborepo_trace_graph::{ChangeKind, ChangeSet, EdgeSpec, MemGraph, RefKind, Verdict};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../turborepo-trace-testkit/fixtures/frameworks")
        .join(name)
        .canonicalize()
        .expect("fixture exists")
}

fn redact(s: String, root: &Path) -> String {
    s.replace(&*root.to_string_lossy(), "<fixture>")
}

fn snap(name: &str, v: serde_json::Value, root: &Path) {
    insta::assert_snapshot!(
        name,
        redact(serde_json::to_string_pretty(&v).unwrap(), root)
    );
}

/// A graph of the fixture: every source file, with import edges resolved
/// relative to the importer (enough for these fixtures).
fn graph(root: &Path) -> MemGraph {
    let mut files = Vec::new();
    turborepo_trace_frameworks::fsx::walk_files(root, &mut |p| {
        let e = p.extension().and_then(|e| e.to_str()).unwrap_or("");
        if matches!(e, "ts" | "tsx" | "js" | "mjs" | "css" | "json") {
            files.push(p.to_path_buf());
        }
    });
    let mut b = MemGraph::builder();
    b.package("next-app", None, "");
    let ids: Vec<_> = files
        .iter()
        .map(|f| b.file(&f.strip_prefix(root).unwrap().to_string_lossy()))
        .collect();
    let r = SimpleResolver {
        extensions: ["ts", "tsx", "js", "mjs", "css"].map(String::from).to_vec(),
    };
    for (i, f) in files.iter().enumerate() {
        if f.extension().is_some_and(|e| e == "css" || e == "json") {
            continue;
        }
        let js = JsFile::read(f).unwrap();
        for imp in &js.imports {
            let to = turborepo_trace_frameworks::ModuleResolver::resolve(
                &r,
                &imp.module,
                f.parent().unwrap(),
            );
            if let Some(to) = to
                && let Some(j) = files.iter().position(|x| *x == to)
            {
                b.add_edge(
                    ids[i],
                    EdgeSpec::new(Some(ids[j]), RefKind::Import).specifier(imp.module.clone()),
                );
            }
        }
    }
    b.build().unwrap()
}

#[test]
fn route_table() {
    let root = fixture("next-app");
    let app = NextApp::load(&root);
    let routes: Vec<_> = app
        .table
        .routes
        .iter()
        .map(|r| {
            json!({
                "id": r.id, "pattern": r.pattern, "kind": r.kind, "router": r.router,
                "slot": r.slot, "intercept": r.intercept, "metadata": r.metadata,
            })
        })
        .collect();
    snap(
        "route_table",
        json!({
            "app_dir": app.table.app_dir, "pages_dir": app.table.pages_dir,
            "page_exts": app.table.page_exts, "routes": routes,
            "conflicts": app.table.conflicts, "root_layouts": app.table.root_layouts(),
            "pages_special": app.table.pages_special,
        }),
        &root,
    );
    // Private folders are not routes.
    assert!(
        !app.table
            .routes
            .iter()
            .any(|r| r.file.contains("_components"))
    );
    // The app/pages `/about` pair is reported.
    assert_eq!(app.table.conflicts.len(), 1);
}

#[test]
fn segment_closures() {
    let root = fixture("next-app");
    let app = NextApp::load(&root);
    let mut out = serde_json::Map::new();
    for id in [
        "page:app/[team]/settings/page.tsx",
        "page:app/[team]/page.tsx",
        "page:app/(marketing)/about/page.tsx",
        "page:app/blog/[slug]/page.tsx",
        "route:app/api/health/route.ts",
        "pages:pages/legacy.tsx",
    ] {
        let r = app.table.route(id).unwrap_or_else(|| panic!("{id}"));
        out.insert(
            id.into(),
            serde_json::to_value(app.table.segment_closure(r)).unwrap(),
        );
    }
    snap("segment_closures", serde_json::Value::Object(out), &root);
    let settings = app.table.segment_closure(
        app.table
            .route("page:app/[team]/settings/page.tsx")
            .unwrap(),
    );
    // The sidebar slot renders its matching page, the modal slot its default; the
    // intercepting photo modal is only on soft navigation.
    assert!(
        settings
            .files
            .contains(&"app/[team]/@sidebar/settings/page.tsx".to_string())
    );
    assert!(
        settings
            .files
            .contains(&"app/[team]/@modal/default.tsx".to_string())
    );
    assert!(settings.soft.iter().any(|f| f.contains("(.)photo")));
}

#[test]
fn next_config() {
    let root = fixture("next-app");
    let app = NextApp::load(&root);
    let mut c = serde_json::to_value(&app.config).unwrap();
    c.as_object_mut().unwrap().remove("files");
    c.as_object_mut().unwrap().remove("code_files");
    snap("next_config", c, &root);
    assert!(app.config.wrapped);
    assert_eq!(app.config.redirects.len(), 3, "{:?}", app.config.redirects);
    assert_eq!(app.config.rewrites.before_files.len(), 1);
    assert_eq!(app.config.rewrites.after_files.len(), 2);
    assert!(
        app.config
            .files
            .iter()
            .any(|f| f.ends_with("redirects.mjs"))
    );

    let pages = NextApp::load(&fixture("next-pages"));
    assert_eq!(pages.config.base_path.as_deref(), Some("/shop"));
    assert_eq!(pages.config.i18n.as_ref().unwrap().locales, ["en", "de"]);
    // An env-dependent redirect list: kept, marked conditional.
    assert!(pages.config.redirects.iter().all(|r| r.conditional));
}

fn match_table(app: &NextApp, urls: &[&str]) -> serde_json::Value {
    let m = app.matcher();
    let rows: Vec<_> = urls
        .iter()
        .map(|u| {
            let r = m.match_str(u).unwrap();
            json!({
                "url": u, "outcome": r.outcome, "target": r.target, "routes": r.routes,
                "candidates": r.candidates, "ambiguities": r.ambiguities, "via": r.via,
            })
        })
        .collect();
    serde_json::Value::Array(rows)
}

#[test]
fn matcher() {
    let root = fixture("next-app");
    let app = NextApp::load(&root);
    let v = match_table(
        &app,
        &[
            "/",
            "/about",
            "/pricing?x=1",
            "/blog/new",
            "/blog/hello",
            "/docs-local/a/b",
            "/shop",
            "/shop/red/xl",
            "/acme",
            "/acme/settings",
            "/dashboard/settings",
            "/home",
            "/old-blog/first",
            "/beta",
            "/docs/intro",
            "/team-alias/acme",
            "/guides/intro",
            "/help",
            "/favicon.ico",
            "/api/teams/t1/members",
            "/api/v1/projects",
            "/sitemap.xml",
            "/legacy",
            "/api/hello",
            "https://example.com/x",
            "/nope/deep/path",
        ],
    );
    snap("matcher", v, &root);

    // Templates: `/${team}/settings` is ambiguous with `/dashboard/settings`.
    let m = app.matcher();
    let p = turborepo_trace_frameworks::nextjs::urlpat::UrlPat::from_pieces(&[
        turborepo_trace_frameworks::nextjs::urlpat::Piece::Lit("/".into()),
        turborepo_trace_frameworks::nextjs::urlpat::Piece::Wild {
            multi: false,
            reason: "team".into(),
            src: Default::default(),
        },
        turborepo_trace_frameworks::nextjs::urlpat::Piece::Lit("/settings".into()),
    ])
    .unwrap();
    let r = m.match_pat(&p, false);
    assert_eq!(r.routes, ["page:app/[team]/settings/page.tsx"]);
    assert!(
        r.candidates
            .contains(&"page:app/dashboard/settings/page.tsx".to_string())
    );
    assert!(r.ambiguities.iter().any(|a| matches!(
        a,
        turborepo_trace_frameworks::nextjs::matcher::Ambiguity::ParamMayEqualStatic { .. }
    )));

    let proot = fixture("next-pages");
    let pages = NextApp::load(&proot);
    let v = match_table(
        &pages,
        &[
            "/shop",
            "/shop/de/product/1",
            "/shop/product/new",
            "/shop/p/7",
            "/shop/a/b/c",
            "/shop/api/cart",
            "/product/1",
        ],
    );
    snap("matcher_pages", v, &proot);
}

#[test]
fn framework_inputs() {
    let root = fixture("next-app");
    let app = NextApp::load(&root);
    let mut v = serde_json::to_value(app.framework_inputs()).unwrap();
    // Probes depend on what is absent; keep the snapshot readable.
    let probes = v["projects"][0]["global"]["probes"]
        .as_array()
        .map_or(0, |a| a.len());
    v["projects"][0]["global"]["probes"] = json!(probes);
    snap("framework_inputs", v, &root);
    let g: Vec<_> = app
        .globals
        .files
        .iter()
        .map(|f| {
            f.file
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    for want in [
        "next.config.ts",
        "redirects.mjs",
        "middleware.ts",
        "instrumentation.ts",
        "app/layout.tsx",
        "app/globals.css",
        "microfrontends.json",
        "pages/_app.tsx",
        "postcss.config.mjs",
    ] {
        assert!(g.iter().any(|x| x == want), "{want} in {g:?}");
    }
}

#[test]
fn fetch_sites() {
    let root = fixture("next-app");
    let app = NextApp::load(&root);
    let files: Vec<PathBuf> = [
        "components/members.tsx",
        "components/nav.tsx",
        "lib/api.ts",
        "app/[team]/page.tsx",
    ]
    .iter()
    .map(|f| root.join(f))
    .collect();
    let sites = select::scan_sites(&files, &default_url_apis(), &SimpleResolver::default());
    let resolved = select::resolve_sites(&app, &sites, &RequestContext::default());
    let rows: Vec<_> = resolved
        .iter()
        .map(|r| {
            json!({
                "file": r.site.file, "line": r.site.line, "api": r.site.api, "kind": r.site.kind,
                "urls": r.site.urls, "unresolved": r.site.unresolved, "routes": r.routes,
                "external": r.external, "rewritten": r.rewritten,
            })
        })
        .collect();
    snap("fetch_sites", serde_json::Value::Array(rows), &root);
    let stats = select::site_stats(&app, &resolved);
    // SWR `/api/teams/*/members`, `fetch('/api/health')`, and
    // `getJson('/api/health')` through the derived wrapper.
    assert_eq!(stats.to_handler, 3, "{stats:?}");
    assert!(stats.external >= 2, "{stats:?}");
    // Only `fetch(window.__ENDPOINT__)`: `fetch(endpoint)` forwards to the local
    // `ping`, the SWR fetcher's `fetch(u)` to its caller, `getJson`'s
    // `fetch(url)` to `getJson`.
    assert_eq!(stats.unresolved, 1, "{stats:?}");
    // ... and `postJson`'s and `getText`'s (return-type annotated) `fetch(url)`.
    assert_eq!(stats.forwarded, 5, "{stats:?}");
    assert!(
        resolved
            .iter()
            .any(|r| r.site.api.starts_with("getJson (wraps fetch)")
                && r.routes == ["route:app/api/health/route.ts"])
    );
    let missing = resolved
        .iter()
        .find(|r| r.site.urls.iter().any(|u| u.to_string() == "/api/settings"))
        .expect("/api/settings site");
    assert!(missing.routes.is_empty(), "{missing:?}");
    assert_eq!(missing.not_found, 1, "{missing:?}");
}

fn specs(root: &Path) -> Vec<SpecVisits> {
    let (inputs, _) = playwright::load_config(&root.join("playwright.config.ts"));
    let resolver = SimpleResolver::default();
    let files = Files::new(&resolver);
    playwright::discover_specs(&inputs)
        .iter()
        .map(|s| extract_visits(s, &files, &ExtractOptions::default()))
        .collect()
}

#[test]
fn playwright_config_and_visits() {
    let root = fixture("next-app");
    let pw: &dyn Framework = framework("playwright").unwrap();
    assert_eq!(
        pw.config_for(&root.join("e2e/home.spec.ts")),
        Some(root.join("playwright.config.ts"))
    );
    let (inputs, base) = playwright::load_config(&root.join("playwright.config.ts"));
    assert_eq!(inputs.projects.len(), 2);
    assert_eq!(base.0[0].1.as_deref(), Some("http://localhost:3000"));
    let found: Vec<String> = playwright::discover_specs(&inputs)
        .iter()
        .map(|p| {
            p.strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert!(!found.iter().any(|f| f.contains("helpers")));
    assert_eq!(found.len(), 8, "{found:?}");
    let ti = Playwright.test_inputs(
        &inputs.projects[0],
        &root.join("e2e/home.spec.ts"),
        b"",
        &SimpleResolver::default(),
    );
    assert_eq!(
        ti.memberships[0].dir,
        root.join("e2e/home.spec.ts-snapshots")
    );

    let s = specs(&root);
    snap("visits", serde_json::to_value(&s).unwrap(), &root);
    let by = |n: &str| s.iter().find(|v| v.spec.ends_with(n)).unwrap();
    assert!(by("home.spec.ts").complete());
    assert!(by("team.spec.ts").complete());
    assert!(!by("dynamic.spec.ts").complete());
    assert!(
        by("annotated.spec.ts").complete(),
        "the annotation replaces extraction"
    );
    assert!(
        by("blog.spec.ts")
            .helper_files
            .iter()
            .any(|f| f.ends_with("helpers.ts"))
    );
}

/// `(path, before, after)`.
type Versions = (String, Option<Vec<u8>>, Option<Vec<u8>>);
struct Mem(Vec<Versions>);
impl Contents for Mem {
    fn before(&self, p: &str) -> Option<Vec<u8>> {
        self.0.iter().find(|x| x.0 == p).and_then(|x| x.1.clone())
    }
    fn after(&self, p: &str) -> Option<Vec<u8>> {
        self.0.iter().find(|x| x.0 == p).and_then(|x| x.2.clone())
    }
}

/// Contents with unchanged class tokens (the file itself before and after), so
/// that Tailwind does not make every change global in tests about other inputs.
struct SameTokens(PathBuf);
impl Contents for SameTokens {
    fn before(&self, p: &str) -> Option<Vec<u8>> {
        std::fs::read(self.0.join(p)).ok()
    }
    fn after(&self, p: &str) -> Option<Vec<u8>> {
        std::fs::read(self.0.join(p)).ok()
    }
}

struct Setup {
    root: PathBuf,
    app: NextApp,
    g: MemGraph,
    specs: Vec<SpecVisits>,
    sites: Vec<select::ResolvedSite>,
    same: SameTokens,
}

fn setup() -> Setup {
    let root = fixture("next-app");
    let app = NextApp::load(&root);
    let g = graph(&root);
    let specs = specs(&root);
    let mut files = Vec::new();
    turborepo_trace_frameworks::fsx::walk_files(&root, &mut |p| {
        if p.extension().is_some_and(|e| e == "ts" || e == "tsx")
            && !p.to_string_lossy().contains("/e2e/")
        {
            files.push(p.to_path_buf());
        }
    });
    let raw = select::scan_sites(&files, &default_url_apis(), &SimpleResolver::default());
    let sites = select::resolve_sites(&app, &raw, &RequestContext::default());
    let same = SameTokens(root.clone());
    Setup {
        root,
        app,
        g,
        specs,
        sites,
        same,
    }
}

fn run<'b>(
    s: &'b Setup,
    changes: &[(ChangeKind, &str)],
    f: impl FnOnce(&mut SelectOptions<'b>),
) -> Vec<(String, Option<Reason>)> {
    let mut cs = ChangeSet::new();
    for (k, p) in changes {
        cs.push(*k, *p);
    }
    let mut opts = SelectOptions::new(&s.root);
    opts.fetch = Some(&s.sites);
    opts.tailwind = TailwindMode::Tokens;
    opts.contents = Some(&s.same);
    f(&mut opts);
    let r = select_e2e(&cs, &s.specs, &s.app, &s.g, &opts);
    r.specs
        .into_iter()
        .filter(|v| v.verdict != Verdict::Unaffected)
        .map(|v| {
            (
                v.spec.file_name().unwrap().to_string_lossy().into_owned(),
                v.reason,
            )
        })
        .collect()
}

fn names(v: &[(String, Option<Reason>)]) -> BTreeSet<&str> {
    v.iter().map(|(n, _)| n.as_str()).collect()
}

const ALL: [&str; 8] = [
    "annotated.spec.ts",
    "api.spec.ts",
    "blog.spec.ts",
    "dynamic.spec.ts",
    "home.spec.ts",
    "mfe.spec.ts",
    "route.spec.ts",
    "team.spec.ts",
];

#[test]
fn select_route_content() {
    let s = setup();
    use ChangeKind::*;
    // A component on the team settings page: the team spec, and the spec that may
    // visit any route.
    let r = run(&s, &[(Modified, "components/members.tsx")], |_| {});
    assert_eq!(
        names(&r),
        BTreeSet::from(["team.spec.ts", "dynamic.spec.ts", "route.spec.ts"]),
        "{r:?}"
    );
    let team = r.iter().find(|x| x.0 == "team.spec.ts").unwrap();
    assert!(matches!(&team.1, Some(Reason::Route(_))), "{team:?}");
    // A page reached through a redirect and a helper.
    let r = run(&s, &[(Modified, "components/price.tsx")], |_| {});
    assert_eq!(
        names(&r),
        BTreeSet::from(["blog.spec.ts", "dynamic.spec.ts"]),
        "{r:?}"
    );
    // The spec itself.
    let r = run(&s, &[(Modified, "e2e/home.spec.ts")], |_| {});
    assert_eq!(names(&r), BTreeSet::from(["home.spec.ts"]));
    // A page object.
    let r = run(&s, &[(Modified, "e2e/pages/team-page.ts")], |_| {});
    assert_eq!(names(&r), BTreeSet::from(["team.spec.ts"]));
    // Unrelated file.
    assert!(run(&s, &[(Modified, "README.md")], |_| {}).is_empty());
}

#[test]
fn select_handlers_via_fetch() {
    let s = setup();
    use ChangeKind::*;
    // `/api/health` is fetched by members.tsx (team settings) and requested by
    // api.spec.
    let r = run(&s, &[(Modified, "app/api/health/route.ts")], |_| {});
    let n = names(&r);
    assert!(
        n.contains("api.spec.ts") && n.contains("team.spec.ts"),
        "{r:?}"
    );
    assert!(!n.contains("home.spec.ts"), "{r:?}");
    let team = r.iter().find(|x| x.0 == "team.spec.ts").unwrap();
    // members.tsx also has an unresolved fetch: either reason is a correct
    // explanation.
    assert!(
        matches!(
            &team.1,
            Some(Reason::FetchedRoute(_) | Reason::UnresolvedFetch)
        ),
        "{team:?}"
    );
    // A handler nothing references statically: only specs reaching unresolved fetch
    // sites. (route.spec reaches the settings page through `waitForURL`,
    // dynamic.spec may visit any route.)
    let r = run(&s, &[(Modified, "pages/api/hello.ts")], |_| {});
    assert_eq!(
        names(&r),
        BTreeSet::from(["team.spec.ts", "route.spec.ts", "dynamic.spec.ts"]),
        "{r:?}"
    );
    let r = run(&s, &[(Modified, "pages/api/hello.ts")], |o| {
        o.unresolved_fetch_any_handler = false
    });
    assert!(r.is_empty(), "{r:?}");
}

#[test]
fn select_globals() {
    let s = setup();
    use ChangeKind::*;
    for p in [
        "app/layout.tsx",
        "next.config.ts",
        "redirects.mjs",
        "middleware.ts",
        "app/globals.css",
        "public/favicon.ico",
        "microfrontends.json",
    ] {
        let r = run(&s, &[(Modified, p)], |_| {});
        assert_eq!(names(&r), BTreeSet::from(ALL), "{p}: {r:?}");
    }
    let r = run(&s, &[(Added, ".env.local")], |_| {});
    assert_eq!(names(&r).len(), 8);
    let r = run(&s, &[(Added, "proxy.ts")], |_| {});
    assert_eq!(names(&r).len(), 8, "probe on a new proxy.ts");
}

#[test]
fn select_route_table_changes() {
    let s = setup();
    use ChangeKind::*;
    // A new static route under /blog competes with /blog/[slug] where the slug is
    // unknown (dynamic.spec.ts). blog.spec.ts visits `blogPath('hello')`, which
    // evaluates to `/blog/hello` through the helper's parameter and cannot hit
    // `/blog/featured`.
    let r = run(&s, &[(Added, "app/blog/featured/page.tsx")], |_| {});
    assert_eq!(names(&r), BTreeSet::from(["dynamic.spec.ts"]), "{r:?}");
    // A new layout below [team] wraps the team routes.
    let r = run(&s, &[(Added, "app/[team]/settings/loading.tsx")], |_| {});
    assert_eq!(
        names(&r),
        BTreeSet::from(["team.spec.ts", "dynamic.spec.ts", "route.spec.ts"]),
        "{r:?}"
    );
    // Membership mode: any route file selects everything.
    let r = run(&s, &[(Added, "app/blog/featured/page.tsx")], |o| {
        o.route_table = RouteTableMode::Membership
    });
    assert_eq!(names(&r).len(), 8);
}

#[test]
fn select_ambiguity_policy() {
    let s = setup();
    use ChangeKind::*;
    // `/${slug}/settings` may hit /dashboard/settings if slug == "dashboard".
    let r = run(&s, &[(Modified, "app/dashboard/settings/page.tsx")], |_| {});
    assert!(names(&r).contains("team.spec.ts"), "{r:?}");
    let r = run(&s, &[(Modified, "app/dashboard/settings/page.tsx")], |o| {
        o.ambiguity = AmbiguityPolicy::Heuristic
    });
    assert!(!names(&r).contains("team.spec.ts"), "{r:?}");
}

#[test]
fn select_tailwind() {
    let s = setup();
    use ChangeKind::*;
    let path = "components/price.tsx";
    let old = std::fs::read(s.root.join(path)).unwrap();
    // Strict (the default): a scanned file regenerates the stylesheet → every spec.
    let r = run(&s, &[(Modified, path)], |o| {
        o.tailwind = TailwindMode::Strict;
        o.contents = None;
    });
    assert_eq!(names(&r).len(), 8, "{r:?}");
    let same = String::from_utf8(old.clone())
        .unwrap()
        .replace("$1", "$2")
        .into_bytes();
    let new_cls = String::from_utf8(old.clone())
        .unwrap()
        .replace("text-lg", "text-xl")
        .into_bytes();
    let c1 = Mem(vec![(path.into(), Some(old.clone()), Some(same))]);
    let r = run(&s, &[(Modified, path)], |o| {
        o.tailwind = TailwindMode::Tokens;
        o.contents = Some(&c1);
    });
    assert_eq!(
        names(&r),
        BTreeSet::from(["blog.spec.ts", "dynamic.spec.ts"]),
        "{r:?}"
    );
    let c2 = Mem(vec![(path.into(), Some(old), Some(new_cls))]);
    let r = run(&s, &[(Modified, path)], |o| {
        o.tailwind = TailwindMode::Tokens;
        o.contents = Some(&c2);
    });
    assert_eq!(names(&r).len(), 8, "{r:?}");
    assert!(r.iter().any(|x| matches!(x.1, Some(Reason::TailwindCss))));
}

#[test]
fn select_report_snapshot() {
    let s = setup();
    let mut cs = ChangeSet::new();
    cs.push(
        ChangeKind::Modified,
        "app/api/teams/[teamId]/members/route.ts",
    );
    let mut opts = SelectOptions::new(&s.root);
    opts.fetch = Some(&s.sites);
    opts.tailwind = TailwindMode::Tokens;
    opts.contents = Some(&s.same);
    let r = select_e2e(&cs, &s.specs, &s.app, &s.g, &opts);
    snap(
        "select_members_handler",
        serde_json::to_value(&r).unwrap(),
        &s.root,
    );
    // route.spec mocks /api/teams/*/members (page.route); team.spec renders
    // members.tsx.
    let sel: BTreeSet<_> = r
        .selected()
        .map(|v| v.spec.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert!(
        sel.contains("route.spec.ts") && sel.contains("team.spec.ts"),
        "{sel:?}"
    );
}
