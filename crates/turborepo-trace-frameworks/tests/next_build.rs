//! The Next.js build-output reader over `fixtures/next-build/lab`: a trimmed
//! copy of a real Turbopack build of the lab app
//! (the development repository's `scripts/next-build-lab/mkfixture.mjs`; Next
//! 16.4.5).

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use turborepo_trace_frameworks::nextjs::build::manifest::{
    BuildRouteKind, BuildRouter, Bundler, NextBuild, Runtime,
};

fn lab() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../turborepo-trace-testkit/fixtures/next-build/lab")
}

fn load() -> NextBuild {
    let p = lab();
    NextBuild::load_dist(&p, &p.join("dot-next")).expect("fixture build")
}

#[test]
fn reads_route_entries() {
    let b = load();
    assert_eq!(b.build_id, "Al-SShdjXUxDRuupQCSZm");
    assert_eq!(b.bundler, Bundler::Turbopack);
    assert!(b.warnings.is_empty(), "{:?}", b.warnings);
    let names: Vec<(&str, BuildRouter, BuildRouteKind)> = b
        .routes
        .iter()
        .map(|r| (r.pathname.as_str(), r.router, r.kind))
        .collect();
    use BuildRouteKind::{Handler, Page};
    use BuildRouter::{App, Pages};
    assert_eq!(
        names,
        vec![
            ("/404", Pages, Page),
            ("/_app", Pages, Page),
            ("/_document", Pages, Page),
            ("/_error", Pages, Page),
            ("/_not-found", App, Page),
            ("/about", App, Page),
            ("/api/hello", App, Handler),
            ("/api/old", Pages, Handler),
            ("/api/report-data", App, Handler),
            ("/blog/[slug]", App, Page),
            ("/dashboard/settings", App, Page),
            ("/legacy", Pages, Page),
            ("/legacy/[id]", Pages, Page),
            // The intercepting route keeps its marker; the slot is not part of the pathname.
            ("/photos/(.)photo/[id]", App, Page),
            // Route groups are not part of the pathname.
            ("/pricing", App, Page),
        ]
    );
    assert!(b.routes.iter().all(|r| r.runtime == Runtime::Node));
    assert_eq!(b.public_routes().count(), b.routes.len() - 3);
}

#[test]
fn reads_what_an_entry_loads() {
    let b = load();
    let route = |p: &str| b.routes.iter().find(|r| r.pathname == p).unwrap();

    let about = route("/about");
    assert_eq!(about.key, "/about/page");
    assert_eq!(about.entry, "server/app/about/page.js");
    assert!(about.trace.present);
    // Server chunks inside the dist dir; library files outside it carry the trace's
    // hash.
    assert!(
        about
            .trace
            .inside
            .iter()
            .all(|f| f.starts_with("server/") && b.files.contains(f)),
        "{:?}",
        about.trace.inside
    );
    assert!(about.trace.inside.iter().any(|f| f.contains("chunks/ssr/")));
    assert!(
        about
            .trace
            .external
            .iter()
            .all(|f| f.path.starts_with("node_modules/") && f.hash.is_some()),
        "{:?}",
        about.trace.external
    );
    // Client files: bootstrap chunks, the layout's chunk, and both stylesheets
    // (global + module).
    assert_eq!(
        about.client_manifest.as_deref(),
        Some("server/app/about/page_client-reference-manifest.js")
    );
    assert!(about.client_files.iter().all(|f| b.files.contains(f)));
    let css: Vec<&String> = about
        .client_files
        .iter()
        .filter(|f| f.ends_with(".css"))
        .collect();
    assert_eq!(css.len(), 2, "{css:?}");
    for f in &b.app_bootstrap {
        assert!(about.client_files.contains(f), "{f}");
    }

    // A lazily loaded chunk (`next/dynamic`) comes from the entry's loadable
    // manifest.
    let pricing = route("/pricing");
    assert_eq!(pricing.key, "/(marketing)/pricing/page");
    let lazy = pricing
        .client_files
        .iter()
        .filter(|f| !about.client_files.contains(f))
        .count();
    assert!(lazy >= 1, "{:?}", pricing.client_files);

    // Handlers have no client files.
    assert!(route("/api/hello").client_files.is_empty());
    assert!(route("/api/old").client_files.is_empty());

    // Pages Router: the page's chunk plus `_app` and the polyfills.
    let legacy = route("/legacy");
    assert!(!legacy.client_files.is_empty());
    for f in &b.pages_bootstrap {
        assert!(legacy.client_files.contains(f), "{f}");
    }
}

#[test]
fn reads_actions_prerenders_rules_and_globals() {
    let b = load();
    let ids: Vec<String> = b.actions.iter().map(|a| a.stable_id()).collect();
    assert!(
        ids.contains(&"app/dashboard/settings/actions.ts#saveName".to_string()),
        "{ids:?}"
    );
    let save = b
        .actions
        .iter()
        .find(|a| a.exported_name == "saveName")
        .unwrap();
    assert_eq!(save.workers, vec!["app/dashboard/settings/page"]);
    assert_eq!(save.id.len(), 42);

    // Prerendered paths point back at their route.
    let of = |p: &str| {
        b.prerenders
            .iter()
            .find(|x| x.path == p)
            .map(|x| x.src_route.as_str())
    };
    assert_eq!(of("/blog/first"), Some("/blog/[slug]"));
    assert_eq!(of("/about"), Some("/about"));
    let slug = b
        .routes
        .iter()
        .find(|r| r.pathname == "/blog/[slug]")
        .unwrap();
    let files = b.prerender_files(slug);
    assert!(files.contains(&"server/app/blog/first.html".to_string()));
    assert!(files.contains(&"server/app/blog/second.rsc".to_string()));
    assert!(files.iter().any(|f| f.contains("first.segments/")));
    let legacy_id = b
        .routes
        .iter()
        .find(|r| r.pathname == "/legacy/[id]")
        .unwrap();
    assert!(
        b.prerender_files(legacy_id)
            .contains(&"server/pages/legacy/1.html".to_string())
    );

    // Rules as the build evaluated them, with their compiled regexes.
    let rule = |phase: &str, source: &str| {
        b.routing
            .rules
            .iter()
            .find(|r| r.phase == phase && r.source == source)
    };
    let redirect = rule("redirect", "/old-about").expect("redirect");
    assert_eq!(redirect.destination.as_deref(), Some("/about"));
    assert!(!redirect.internal);
    assert!(rule("redirect", "/:path+/").unwrap().internal);
    assert!(rule("rewrite:afterFiles", "/p/:slug").is_some());
    assert!(rule("header", "/about").is_some());
    // The interception rewrite, with its `next-url` condition.
    let icpt = rule("rewrite:beforeFiles", "/photos/photo/:nxtPid").expect("interception");
    assert_eq!(icpt.raw["has"][0]["key"], "next-url");
    assert!(
        b.routing
            .dynamic_routes
            .iter()
            .any(|r| r.page == "/blog/[slug]" && r.regex.starts_with("^/blog/"))
    );
    assert!(b.routing.static_routes.iter().any(|r| r.page == "/about"));

    // The Node middleware (`proxy.ts`): its entry, traced chunks and matcher.
    let mw = b
        .globals
        .iter()
        .find(|g| g.name == "middleware")
        .expect("middleware");
    assert!(mw.files.contains(&"server/middleware.js".to_string()));
    assert!(mw.files.iter().any(|f| f.starts_with("server/chunks/")));
    assert_eq!(
        mw.matchers[0]["originalSource"],
        "/((?!_next/static|_next/image|favicon.ico).*)"
    );
    assert!(b.config.get("basePath").is_some());
    assert!(b.images.get("images").is_some());
}

#[test]
fn inventory_excludes_non_output() {
    let b = load();
    assert!(b.files.contains("BUILD_ID"));
    assert!(b.files.contains("routes-manifest.json"));
    assert!(
        b.files
            .iter()
            .all(|f| !f.ends_with(".map") && !f.starts_with("cache/"))
    );
    assert!(NextBuild::load_dist(&lab(), &lab().join("missing")).is_err());
}
