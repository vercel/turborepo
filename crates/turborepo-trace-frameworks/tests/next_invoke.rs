//! What routes invoke (`nextjs::invoke`) over
//! `fixtures/frameworks/next-invoke`: sites per idiom, wrapper derivation
//! across files, and the per-route relation.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use serde_json::json;
use turborepo_trace_frameworks::{
    common::SimpleResolver,
    nextjs::{
        NextApp,
        fetch::default_url_apis,
        invoke::{
            self, InvokeKind, InvokeRelation, RelationOptions, Scan, SharedRelation, Site,
            SiteTarget, Target,
        },
        jseval::JsFile,
    },
};
use turborepo_trace_graph::{EdgeSpec, MemGraph, RefKind};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../turborepo-trace-testkit/fixtures/frameworks")
        .join(name)
        .canonicalize()
        .expect("fixture exists")
}

fn code_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    turborepo_trace_frameworks::fsx::walk_files(root, &mut |p| {
        if p.extension().is_some_and(|e| e == "ts" || e == "tsx") {
            files.push(p.to_path_buf());
        }
    });
    files.sort();
    files
}

/// A graph of the fixture: import and re-export edges, resolved relative to the
/// importer.
fn graph(root: &Path, files: &[PathBuf]) -> MemGraph {
    let mut b = MemGraph::builder();
    b.package("next-invoke", None, "");
    let ids: Vec<_> = files
        .iter()
        .map(|f| b.file(&f.strip_prefix(root).unwrap().to_string_lossy()))
        .collect();
    let r = SimpleResolver::default();
    for (i, f) in files.iter().enumerate() {
        let js = JsFile::read(f).unwrap();
        let specs = js
            .imports
            .iter()
            .map(|i| (&i.module, RefKind::Import))
            .chain(js.reexports.iter().map(|r| (&r.module, RefKind::ReExport)));
        for (spec, kind) in specs {
            let to =
                turborepo_trace_frameworks::ModuleResolver::resolve(&r, spec, f.parent().unwrap());
            if let Some(to) = to
                && let Some(j) = files.iter().position(|x| *x == to)
            {
                b.add_edge(
                    ids[i],
                    EdgeSpec::new(Some(ids[j]), kind).specifier(spec.clone()),
                );
            }
        }
    }
    b.build().unwrap()
}

struct Setup {
    root: PathBuf,
    scan: Scan,
    shared: SharedRelation,
    rel: InvokeRelation,
}

fn setup() -> Setup {
    setup_of("next-invoke")
}

fn setup_of(name: &str) -> Setup {
    setup_at(fixture(name))
}

fn setup_at(root: PathBuf) -> Setup {
    let files = code_files(&root);
    let app = NextApp::load(&root);
    let scan = invoke::scan(&files, &default_url_apis(), &SimpleResolver::default());
    let g = graph(&root, &files);
    let opts = RelationOptions {
        graph_root: root.clone(),
        ..RelationOptions::default()
    };
    let trpc = scan.trpc_index(&app, &root);
    let shared = invoke::shared_relation(&app, &g, &scan.sites, &scan.strings, &trpc, &opts);
    let rel = shared.expand();
    Setup {
        root,
        scan,
        shared,
        rel,
    }
}

fn rel_file(root: &Path, s: &Site) -> String {
    s.file
        .strip_prefix(root)
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

fn target_text(t: &SiteTarget) -> String {
    match t {
        SiteTarget::Url(u) => u.to_string(),
        SiteTarget::Procedure(p) => format!("procedure {p}"),
        SiteTarget::Action { file, export } => format!(
            "action {}#{export}",
            file.file_name().unwrap().to_string_lossy()
        ),
        SiteTarget::Unknown(r) => format!("unknown {r}"),
    }
}

/// The sites of one file as `line kind method via → targets` lines.
fn sites_of(s: &Setup, file: &str) -> Vec<String> {
    s.scan
        .sites
        .iter()
        .filter(|x| rel_file(&s.root, x) == file)
        .map(|x| {
            let targets: Vec<String> = x.targets.iter().map(target_text).collect();
            format!(
                "{} {} {} [{}] -> {}",
                x.line,
                x.kind.as_str(),
                x.method.as_deref().unwrap_or("-"),
                x.via.join(" > "),
                targets.join(" | ")
            )
        })
        .collect()
}

/// The dependency targets of a route: `kind target` lines.
fn invoked(s: &Setup, route: &str) -> Vec<String> {
    let mut v: Vec<String> = s.rel.routes[route]
        .iter()
        .filter(|i| i.kind.is_dependency())
        .map(|i| {
            let t = match &i.target {
                Target::Route(k) => format!("route {k}"),
                Target::Action(a) => format!("action {a}"),
                Target::Procedure { path, file, .. } => {
                    format!("procedure {path} in {}", file.as_deref().unwrap_or("?"))
                }
                Target::External { origin, pattern } => format!("external {origin}{pattern}"),
                Target::Unknown(r) => format!("unknown {r}"),
            };
            format!("{} {t}", i.kind.as_str())
        })
        .collect();
    v.sort();
    v.dedup();
    v
}

#[test]
fn sites_snapshot() {
    let s = setup();
    let rows: Vec<_> = s
        .scan
        .sites
        .iter()
        .map(|x| {
            let targets: Vec<String> = x.targets.iter().map(target_text).collect();
            json!({
                "file": rel_file(&s.root, x), "line": x.line, "kind": x.kind,
                "method": x.method, "via": x.via, "targets": targets,
            })
        })
        .collect();
    insta::assert_snapshot!("invoke_sites", serde_json::to_string_pretty(&rows).unwrap());
}

#[test]
fn relation_snapshot() {
    let s = setup();
    insta::assert_snapshot!(
        "invoke_relation",
        serde_json::to_string_pretty(&s.rel).unwrap()
    );
}

/// The wrapper that adds a base and an `/api` prefix behind `startsWith`
/// guards, and calls `fetch` from a callback through a second function: its
/// call sites are the sites.
#[test]
fn fetch_wrapper_with_guards() {
    let s = setup();
    // No site inside the wrapper: its callers carry the URLs.
    assert!(sites_of(&s, "lib/api/fetch-api.ts").is_empty());
    let items = sites_of(&s, "components/items.tsx");
    // `apiFetch(API_ROUTES.HEALTH)`: an object-constant member, already under
    // `/api`.
    assert!(
        items.iter().any(|l| l.contains("/api/health")
            && !l.contains("/api/api/")
            && l.contains("[apiFetch > apiFetchInternal > fetch]")),
        "{items:#?}"
    );
    // `apiFetch(`${API_ITEMS_VERSION('v1')}/${id}`, { method: 'DELETE' })`: a
    // function constant, and the method through `{ ...opts, headers }`.
    assert!(
        items
            .iter()
            .any(|l| l.contains(" DELETE ") && l.contains("/api/v1/items/*")),
        "{items:#?}"
    );
    // `apiFetch('/v9/projects')`: the prefix is added; no route serves it.
    let backend = invoked(&s, "app/settings/page");
    assert!(
        backend.contains(&"fetch external /api/v9/projects".to_string()),
        "{backend:#?}"
    );
    assert!(
        backend
            .contains(&"fetch external https://status.example.com/api/v2/status.json".to_string()),
        "{backend:#?}"
    );
}

/// The typed client: a barrel re-export, overloads, a destructured call result,
/// `{param}` placeholders substituted in a loop, and a query string appended.
#[test]
fn typed_client_through_barrel() {
    let s = setup();
    let items = sites_of(&s, "components/items.tsx");
    let typed: Vec<&String> = items.iter().filter(|l| l.contains("typedFetch")).collect();
    assert_eq!(typed.len(), 1, "{items:#?}");
    assert!(typed[0].contains("/api/v1/items/*"), "{typed:#?}");
    assert!(!typed[0].contains("unknown"), "{typed:#?}");
    assert!(typed[0].contains(" GET "), "{typed:#?}");
    let home = invoked(&s, "app/page");
    assert!(
        home.contains(&"fetch route app/api/v1/items/[id]/route".to_string()),
        "{home:#?}"
    );
}

/// SWR: an inline fetcher receives the key; a wrapper hook that forwards key
/// and fetcher binds its callers' fetchers; a fetcher function is applied to
/// the key.
#[test]
fn swr_keys_and_fetchers() {
    let s = setup();
    // `useSWR<{...} | null>(cond ? `/api/teams/${id}?…` : null, async (url) =>
    // fetch(url))`.
    let card = sites_of(&s, "components/team-card.tsx");
    assert_eq!(card.len(), 1, "{card:#?}");
    assert!(
        card[0].contains(" swr ") && card[0].contains("/api/teams/*"),
        "{card:#?}"
    );
    // `useLazySWR(team ? membersKey(team.id) : null, async (url) => fetch(url,
    // …))`.
    let members = sites_of(&s, "components/members.tsx");
    assert_eq!(members.len(), 1, "{members:#?}");
    assert!(members[0].contains("/api/teams/*/members"), "{members:#?}");
    // `useLazySWR(`/api/dash?…`, async (endpoint) => apiFetch(endpoint, …))`.
    let dash = sites_of(&s, "components/dash.tsx");
    assert_eq!(dash.len(), 1, "{dash:#?}");
    assert!(
        dash[0].contains("/api/dash") && !dash[0].contains("unknown"),
        "{dash:#?}"
    );
    // `useLazySWR(url, fetchJobsPage)` with `url = useMemo(() =>
    // `${API_ROUTES.JOBS}?…`)`.
    let jobs = sites_of(&s, "components/jobs.tsx");
    assert_eq!(jobs.len(), 1, "{jobs:#?}");
    assert!(
        jobs[0].contains("/api/jobs") && jobs[0].contains("fetchJobsPage"),
        "{jobs:#?}"
    );
    // `useTypedSWR('/v1/items/{id}', …)`: the hook's fetcher builds the URL from
    // the hook's parameter.
    let items = sites_of(&s, "components/items.tsx");
    assert!(
        items.iter().any(|l| l.contains(" swr ")
            && l.contains("useTypedSWR")
            && l.contains("/api/v1/items/*")),
        "{items:#?}"
    );
    // The wrapper hooks themselves have no site.
    assert!(sites_of(&s, "lib/api/swr.ts").is_empty());

    let team = invoked(&s, "app/teams/[teamId]/page");
    assert_eq!(
        team,
        vec![
            "swr route app/api/teams/[teamId]/members/route".to_string(),
            "swr route app/api/teams/[teamId]/route".to_string(),
        ]
    );
}

/// Unknown URLs stay in the relation, once per site, with the expression.
#[test]
fn unknown_sites() {
    let s = setup();
    let u = sites_of(&s, "components/unknown.tsx");
    assert_eq!(u.len(), 2, "{u:#?}");
    assert!(
        u[0].contains("unknown call: localStorage.getItem('endpoint') [storage]"),
        "{u:#?}"
    );
    assert!(
        u[1].ends_with("unknown call: computeUrl(props.kind)"),
        "{u:#?}"
    );
    let settings = &s.rel.routes["app/settings/page"];
    assert_eq!(
        settings
            .iter()
            .filter(|i| matches!(i.target, Target::Unknown(_)))
            .count(),
        2
    );
    assert_eq!(s.rel.stats.pages_with_unknown, 2);
    // Pages that do not reach the unknown sites have none.
    assert!(
        !s.rel.routes["app/page"]
            .iter()
            .any(|i| matches!(i.target, Target::Unknown(_)))
    );
}

/// A URL that is a prop: the JSX element is the call.
#[test]
fn component_props() {
    let s = setup();
    assert!(sites_of(&s, "components/feed.tsx").is_empty());
    let page = sites_of(&s, "app/settings/page.tsx");
    assert_eq!(page.len(), 1, "{page:#?}");
    assert!(
        page[0].contains(" swr ")
            && page[0].contains("-> /api/health")
            && page[0].contains("[Feed > "),
        "{page:#?}"
    );
    assert!(
        invoked(&s, "app/settings/page").contains(&"swr route app/api/health/route".to_string())
    );
}

/// Redirects are dependencies; links are listed as navigations.
#[test]
fn redirects_and_navigations() {
    let s = setup();
    let login = &s.rel.routes["app/login/page"];
    assert!(login.iter().any(|i| i.kind == InvokeKind::Redirect
        && i.target == Target::Route("app/teams/[teamId]/page".into())));
    assert!(login.iter().any(|i| i.kind == InvokeKind::Navigation
        && i.target == Target::Route("app/settings/page".into())));
}

/// Server actions: every use of an action export, with how it is used; a barrel
/// does not hide the module that declares the action; an inline action belongs
/// to its file.
#[test]
fn server_actions() {
    let s = setup();
    let form = sites_of(&s, "components/team-form.tsx");
    let has = |via: &str, action: &str| {
        form.iter().any(|l| {
            l.contains(&format!("[{via}]")) && l.ends_with(&format!("action actions.ts#{action}"))
        })
    };
    assert!(has("hook:useActionState", "createTeam"), "{form:#?}");
    assert!(has("bind", "renameTeam"), "{form:#?}");
    assert!(has("call", "deleteTeam"), "{form:#?}");
    assert!(has("hook:useAction", "inviteMember"), "{form:#?}");
    // The page with the form: the actions of its closure and its own inline action.
    let page = invoked(&s, "app/teams/new/page");
    for a in ["createTeam", "renameTeam", "deleteTeam", "inviteMember"] {
        assert!(
            page.contains(&format!("action action app/actions.ts#{a}")),
            "{a}: {page:#?}"
        );
    }
    assert!(
        page.contains(&"action action app/teams/new/page.tsx#<inline:subscribe>".to_string()),
        "{page:#?}"
    );
    // Other pages invoke no action.
    assert!(
        !invoked(&s, "app/page")
            .iter()
            .any(|l| l.starts_with("action"))
    );
}

/// TanStack Query: `queryFn` / `mutationFn` bodies are the sites, of kind
/// `query`, and they reach the pages that use the custom hook through the
/// module graph.
#[test]
fn tanstack_query_hooks() {
    let s = setup();
    let hook = sites_of(&s, "lib/use-jobs-query.ts");
    assert_eq!(hook.len(), 3, "{hook:#?}");
    assert!(hook.iter().all(|l| l.contains(" query ")), "{hook:#?}");
    assert!(hook[0].contains("/api/jobs"), "{hook:#?}");
    assert!(
        hook[2].contains(" DELETE ") && hook[2].contains("/api/v1/items/*"),
        "{hook:#?}"
    );
    let routes: Vec<String> = invoked(&s, "app/queries/page")
        .into_iter()
        .filter(|l| l.contains(" route "))
        .collect();
    assert_eq!(
        routes,
        vec![
            "query route app/api/health/route".to_string(),
            "query route app/api/jobs/route".to_string(),
            "query route app/api/v1/items/[id]/route".to_string(),
        ]
    );
}

/// tRPC: procedure paths from client chains and a server-side caller, the
/// router tree across files, and the HTTP entry as the serving route.
#[test]
fn trpc_procedures() {
    let s = setup();
    let procs: Vec<(String, String, String)> = s
        .rel
        .procedures
        .iter()
        .map(|p| (p.path.clone(), p.kind.clone(), p.file.clone()))
        .collect();
    let want = [
        ("greeting", "query", "server/root.ts"),
        ("post.all", "query", "server/routers/post.ts"),
        ("post.comments.add", "mutation", "server/routers/post.ts"),
        ("user.byId", "query", "server/routers/user.ts"),
        ("user.create", "mutation", "server/routers/user.ts"),
        ("user.list", "query", "server/routers/user.ts"),
    ];
    assert_eq!(
        procs,
        want.iter()
            .map(|(a, b, c)| (a.to_string(), b.to_string(), c.to_string()))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        s.rel.trpc_handlers,
        vec!["app/api/trpc/[trpc]/route".to_string()]
    );
    let panel = sites_of(&s, "components/user-panel.tsx");
    let paths: Vec<&str> = panel
        .iter()
        .map(|l| l.rsplit("procedure ").next().unwrap())
        .collect();
    assert_eq!(
        paths,
        [
            "user.byId",
            "post.all",
            "user.create",
            "user.list",
            "post.comments.add"
        ],
        "{panel:#?}"
    );
    let page = &s.rel.routes["app/users/[id]/page"];
    let targets: Vec<(String, Option<String>, Option<String>)> = page
        .iter()
        .filter_map(|i| match &i.target {
            Target::Procedure {
                path,
                file,
                serving,
            } => Some((path.clone(), file.clone(), serving.clone())),
            _ => None,
        })
        .collect();
    assert!(
        targets.contains(&(
            "user.byId".to_string(),
            Some("server/routers/user.ts".to_string()),
            Some("app/api/trpc/[trpc]/route".to_string())
        )),
        "{targets:#?}"
    );
    // The server-side caller.
    assert!(targets.iter().any(|t| t.0 == "greeting"), "{targets:#?}");
    assert_eq!(s.rel.stats.to_procedure, 6);
}

/// The bound of unknown targets: an unevaluable URL reaches only the handler
/// routes that a URL-like string of the page's closure matches.
#[test]
fn unknown_bound() {
    let root = fixture("next-invoke");
    let files = code_files(&root);
    let app = NextApp::load(&root);
    let scan = invoke::scan(&files, &default_url_apis(), &SimpleResolver::default());
    let g = graph(&root, &files);
    let opts = RelationOptions {
        graph_root: root.clone(),
        bound_unknown: true,
        ..RelationOptions::default()
    };
    let trpc = scan.trpc_index(&app, &root);
    let rel = invoke::relation(&app, &g, &scan.sites, &scan.strings, &trpc, &opts);
    // The settings page has two unknown sites; the only URL-like strings of its
    // closure that match a handler are `/api/health` (a prop) and nothing else.
    let bound: Vec<&String> = rel.routes["app/settings/page"]
        .iter()
        .filter(|i| i.via == ["url-like string".to_string()])
        .filter_map(|i| match &i.target {
            Target::Route(k) => Some(k),
            _ => None,
        })
        .collect();
    assert!(bound.is_empty(), "already a site target: {bound:#?}");
    assert!(rel.stats.bounded);
    assert_eq!(rel.stats.pages_with_unknown, 2);
    assert_eq!(rel.stats.bound_added, [0; 6]);
    // In the consumer's relation the bounded unknown (`computeUrl(…)`) is gone; the
    // URL read from storage comes from outside the code and stays unknown
    // (every handler). So do the four of the sources page.
    assert_eq!(rel.stats.pages_unbounded, 2);
    assert_eq!(rel.stats.unbounded_sites, 5);
    let b = invoke::build_relation(&app, &g, &root);
    let unknown: Vec<String> = b
        .routes
        .values()
        .flatten()
        .filter_map(|i| match &i.target {
            turborepo_trace_frameworks::nextjs::build::invokes::InvokeTarget::Unknown(r) => {
                Some(r.clone())
            }
            _ => None,
        })
        .collect();
    assert_eq!(unknown.len(), 5, "{unknown:?}");
    assert!(
        unknown.iter().any(|u| u.contains("localStorage")),
        "{unknown:?}"
    );
    assert!(
        !unknown
            .iter()
            .any(|u| u.contains("computeUrl") || u.contains("pick(")),
        "{unknown:?}"
    );
}

/// Where an unknown URL comes from is found by resolving the expression's
/// identifiers, not by its text: `next` is a search parameter, React state
/// holds what an input event gave it, a page's props are the request; a member
/// that is merely named `query` is none of these.
#[test]
fn unknown_sources() {
    let s = setup();
    let u = sites_of(&s, "components/sources.tsx");
    assert_eq!(u.len(), 3, "{u:#?}");
    assert!(u[0].contains("params.get('next') [request]"), "{u:#?}");
    assert!(u[1].contains("useState('').0 [input]"), "{u:#?}");
    assert!(
        u[2].contains("sessionStorage.getItem('last') [storage]"),
        "{u:#?}"
    );
    // The component's prop is bound at its JSX use, in the page.
    let p = sites_of(&s, "app/sources/page.tsx");
    assert_eq!(p.len(), 2, "{p:#?}");
    // A page is called by the framework: its site is not a wrapper waiting for
    // callers.
    assert!(p[0].contains("searchParams.u [request]"), "{p:#?}");
    assert!(p[1].ends_with("unknown call: pick('a')"), "{p:#?}");
    assert_eq!(s.rel.stats.unknown_sources["request"], 2);
    assert_eq!(s.rel.stats.unknown_sources["storage"], 2);
    assert_eq!(s.rel.stats.unknown_sources["input"], 1);
    // What a function of unknown body returns is the code's.
    assert_eq!(s.rel.stats.unknown_sources["code"], 2);
    assert!(invoke::outside_input("call: x() [storage]"));
    assert!(!invoke::outside_input("member: client.query"));
    assert!(!invoke::outside_input("identifier: url [param]"));
}

/// Where the links and redirects of a route's closure lead: `kind route` lines,
/// ` (wide)` for a target that only an arbitrary middleware prefix reaches.
fn goes_to(s: &Setup, route: &str) -> Vec<String> {
    let mut v: Vec<String> = s.rel.routes[route]
        .iter()
        .filter(|i| matches!(i.kind, InvokeKind::Redirect | InvokeKind::Navigation))
        .filter_map(|i| match &i.target {
            Target::Route(k) => Some(format!(
                "{} {k}{}",
                i.kind.as_str(),
                if i.wide { " (wide)" } else { "" }
            )),
            _ => None,
        })
        .collect();
    v.sort();
    v
}

/// The target of a link or a redirect is requested like a URL a spec names:
/// where the middleware runs for it, it is matched behind the segment the
/// middleware puts in front (`fixtures/frameworks/next-invoke-locale`: every
/// page is under `/[locale]`).
#[test]
fn link_and_redirect_targets_behind_the_middleware_prefix() {
    let s = setup_of("next-invoke-locale");
    let has = |route: &str, line: &str| {
        let v = goes_to(&s, route);
        assert!(v.iter().any(|x| x == line), "{route}: {line} not in {v:#?}");
    };
    let has_not = |route: &str, line: &str| {
        let v = goes_to(&s, route);
        assert!(!v.iter().any(|x| x == line), "{route}: {line} in {v:#?}");
    };
    // `redirect('/login')` and `router.push('/login/verify')`.
    has(
        "app/[locale]/login/verify/page",
        "redirect app/[locale]/login/page",
    );
    has(
        "app/[locale]/login/page",
        "navigation app/[locale]/login/verify/page",
    );
    // A URL with an unknown part: by Next's precedence the poll page behind the
    // locale; the page behind the fixed segment `embed` only if the middleware
    // put `embed` there.
    has(
        "app/[locale]/new/page",
        "navigation app/[locale]/poll/[id]/page",
    );
    has(
        "app/[locale]/new/page",
        "navigation app/embed/poll/[id]/page (wide)",
    );
    has_not(
        "app/[locale]/new/page",
        "navigation app/embed/poll/[id]/page",
    );
    // `<a href="/">` in the layout: the page of `/[locale]`, which the path alone
    // names nowhere.
    has("app/[locale]/login/page", "navigation app/[locale]/page");
    // The middleware's matcher leaves `/docs` alone: the path is matched as it is.
    let docs: Vec<_> = s.rel.routes["app/[locale]/page"]
        .iter()
        .filter(|i| i.url.as_deref() == Some("/docs/intro"))
        .map(|i| (i.target.clone(), i.wide))
        .collect();
    assert_eq!(
        docs,
        [(Target::Route("app/docs/[slug]/page".into()), false)]
    );
    // The middleware is a library's: its file has no redirect or rewrite of its
    // own.
    assert!(s.rel.middleware.is_empty());
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if src.is_dir() {
            copy_dir(&src, &dst);
        } else {
            std::fs::copy(&src, &dst).unwrap();
        }
    }
}

/// A middleware that builds its rewrite target from the request can send a
/// request to any page: the targets by precedence stay, and wide targets, which
/// could say nothing more, are not listed.
#[test]
fn no_wide_targets_under_a_middleware_whose_own_target_is_unknown() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("app");
    copy_dir(&fixture("next-invoke-locale"), &root);
    let root = root.canonicalize().unwrap();
    std::fs::write(
        root.join("middleware.ts"),
        "import { NextResponse, type NextRequest } from 'next/server';\n\nexport function \
         middleware(req: NextRequest) {\n  const url = req.nextUrl.clone();\n  url.pathname = \
         `/${req.cookies.get('locale')?.value ?? 'en'}${url.pathname}`;\n  return \
         NextResponse.rewrite(url);\n}\n",
    )
    .unwrap();
    let s = setup_at(root);
    assert!(
        s.rel
            .middleware
            .iter()
            .any(|i| i.kind == InvokeKind::Redirect && matches!(i.target, Target::Unknown(_)))
    );
    let new = goes_to(&s, "app/[locale]/new/page");
    assert!(
        new.iter()
            .any(|x| x == "navigation app/[locale]/poll/[id]/page"),
        "{new:#?}"
    );
    assert!(!new.iter().any(|x| x.ends_with("(wide)")), "{new:#?}");
    assert!(
        !serde_json::to_string(&s.shared)
            .unwrap()
            .contains("\"wide\"")
    );
    // Without a matcher the middleware runs for `/docs` too.
    let home = goes_to(&s, "app/[locale]/page");
    assert!(
        home.iter().any(|x| x == "navigation app/docs/[slug]/page"),
        "{home:#?}"
    );
}

/// A redirect in a function that a layout calls to load its data belongs to
/// every route below the layout, and leads to the page behind the locale.
#[test]
fn a_redirect_in_a_layouts_loader() {
    let s = setup_of("next-invoke-locale");
    let loader: Vec<_> = sites_of(&s, "lib/loaders.ts");
    assert_eq!(loader.len(), 2, "{loader:#?}");
    assert!(
        loader[1].contains(" redirect ") && loader[1].ends_with("-> /admin-setup"),
        "{loader:#?}"
    );
    for route in ["app/[locale]/admin/page", "app/[locale]/admin/users/page"] {
        let v = goes_to(&s, route);
        for line in [
            "redirect app/[locale]/admin-setup/page",
            "redirect app/[locale]/login/page",
        ] {
            assert!(v.iter().any(|x| x == line), "{route}: {line} not in {v:#?}");
        }
    }
    // Not a route that does not load the loader.
    assert!(
        !goes_to(&s, "app/[locale]/new/page")
            .iter()
            .any(|x| x.starts_with("redirect ")),
    );
}

/// A wide target is marked in the stored relation and is no target for the
/// build fingerprints.
#[test]
fn wide_targets_are_marked_and_left_out_of_the_build_relation() {
    use turborepo_trace_frameworks::nextjs::build::invokes::InvokeTarget;
    let s = setup_of("next-invoke-locale");
    let json = serde_json::to_string(&s.shared).unwrap();
    assert!(json.contains("\"wide\":true"), "{json}");
    assert!(!json.contains("\"wide\":false"), "{json}");
    let back: SharedRelation = serde_json::from_str(&json).unwrap();
    assert_eq!(back, s.shared);
    let b = invoke::to_build(&s.shared);
    let targets: Vec<&str> = b.routes["app/[locale]/new/page"]
        .iter()
        .filter_map(|i| match &i.target {
            InvokeTarget::Route(k) => Some(k.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        targets.contains(&"app/[locale]/poll/[id]/page"),
        "{targets:#?}"
    );
    assert!(
        !targets.contains(&"app/embed/poll/[id]/page"),
        "{targets:#?}"
    );
    // The app without a middleware has no wide target.
    let plain = serde_json::to_string(&setup().shared).unwrap();
    assert!(!plain.contains("\"wide\""));
}

/// A component that is given an `href` is a link: the app's own wrapper of
/// `next/link` (it spreads its props, so its file names no URL) and a component
/// of a package. The site is the element with the attribute.
#[test]
fn an_href_given_to_a_component_is_a_link() {
    let s = setup_of("next-invoke-locale");
    assert_eq!(
        sites_of(&s, "components/nav.tsx"),
        [
            "7 navigation - [<a href>] -> /",
            "8 navigation - [<Link href>] -> /new",
            "9 navigation - [<Button href>] -> /login",
            // `href="#help"` stays on the page: no site.
        ]
    );
    assert!(sites_of(&s, "components/link.tsx").is_empty());
    // The layout renders the navigation: every page below it links to the new-poll
    // page and to the login page.
    for route in ["app/[locale]/admin/users/page", "app/[locale]/page"] {
        let v = goes_to(&s, route);
        for line in [
            "navigation app/[locale]/new/page",
            "navigation app/[locale]/login/page",
        ] {
            assert!(v.iter().any(|x| x == line), "{route}: {line} not in {v:#?}");
        }
    }
    // A page that the layout is not above.
    assert!(goes_to(&s, "app/docs/[slug]/page").is_empty());
}
