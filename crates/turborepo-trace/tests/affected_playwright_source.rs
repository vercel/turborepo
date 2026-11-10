//! `turbotrace affected --tests playwright` with no build and no recording: a
//! changed component selects the specs whose routes load it
//! (`fixtures/next-e2e`).
// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
};

use serde_json::Value;

const ALL: [&str; 5] = [
    "e2e/api.spec.ts",
    "e2e/blog.spec.ts",
    "e2e/dashboard.spec.ts",
    "e2e/home.spec.ts",
    "e2e/pricing.spec.ts",
];

fn copy(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if src.is_dir() {
            copy(&src, &dst);
        } else {
            std::fs::copy(&src, &dst).unwrap();
        }
    }
}

/// A copy of the fixture app as a git repository with one commit (the graph
/// directory must not land in this repository).
fn app() -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../turborepo-trace-testkit/fixtures/next-e2e");
    let root = d.path().join("app");
    copy(&fixture, &root);
    let root = std::fs::canonicalize(root).unwrap();
    std::fs::write(root.join(".gitignore"), ".turbo/trace\n").unwrap();
    git(&root, &["init", "-q"]);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "base"]);
    (d, root)
}

fn git(root: &Path, args: &[&str]) {
    let o = Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(root)
        .output()
        .expect("run git");
    assert!(
        o.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
}

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(support::harness())
        .args(args)
        .current_dir(root)
        .env_remove("TURBO_TRACE_DEBUG_E2E_FOLLOW")
        .output()
        .expect("run turbotrace")
}

fn ok(root: &Path, args: &[&str]) -> String {
    let o = run(root, args);
    assert!(
        o.status.success(),
        "turbotrace {args:?} failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout).unwrap()
}

/// A real change of the code in `rel`: a statement in the first function that
/// returns (a comment would be erased, a top-level statement would be a side
/// effect), or in the first method that awaits (a new export alone changes
/// nothing an importer uses), or a new export, or a line for other files.
fn change(root: &Path, rel: &str) {
    let p = root.join(rel);
    let s = std::fs::read_to_string(&p).unwrap_or_default();
    let code = [".ts", ".tsx", ".mjs", ".js"]
        .iter()
        .any(|e| rel.ends_with(e));
    let new = if code && s.contains("  return ") {
        s.replacen("  return ", "  void 0;\n  return ", 1)
    } else if code && s.contains("    await ") {
        s.replacen("    await ", "    void 0;\n    await ", 1)
    } else if code {
        format!("{s}\nexport const changedForTest = 1;\n")
    } else {
        format!("{s}\nchanged\n")
    };
    std::fs::write(p, new).unwrap();
}

/// The specs selected when the files `changed` are edited since the base
/// commit, with a spec assumed to stay on the pages its URLs name (`--follow
/// calls`): the selection that tells the routes apart.
fn selected(root: &Path, changed: &[&str]) -> Vec<String> {
    selected_with(root, changed, &["--follow", "calls"])
}

/// The same under the default rule: links of the visited pages are followed.
fn selected_by_default(root: &Path, changed: &[&str]) -> Vec<String> {
    selected_with(root, changed, &[])
}

fn selected_with(root: &Path, changed: &[&str], more: &[&str]) -> Vec<String> {
    for c in changed {
        change(root, c);
    }
    let mut args = vec!["affected", "--tests", "playwright", "--since", "HEAD"];
    args.extend(more);
    let out = ok(root, &args);
    // Back to the base commit.
    git(root, &["checkout", "-q", "."]);
    git(root, &["clean", "-q", "-fd", "-e", ".turbo"]);
    out.lines().map(String::from).collect()
}

fn edit(root: &Path, rel: &str, from: &str, to: &str) {
    let p = root.join(rel);
    let s = std::fs::read_to_string(&p).unwrap();
    assert!(s.contains(from), "{rel} has no {from:?}");
    std::fs::write(p, s.replace(from, to)).unwrap();
}

#[test]
fn a_component_of_one_page_selects_the_spec_that_visits_it() {
    let (_d, root) = app();
    assert_eq!(
        selected(&root, &["components/post-body.tsx"]),
        ["e2e/blog.spec.ts"]
    );
    assert_eq!(
        selected(&root, &["components/hero.tsx"]),
        ["e2e/home.spec.ts"]
    );
    // Through the page object, and a component two pages share.
    assert_eq!(
        selected(&root, &["components/button.tsx"]),
        ["e2e/dashboard.spec.ts", "e2e/pricing.spec.ts"]
    );
    assert_eq!(
        selected(&root, &["components/sidebar.tsx"]),
        ["e2e/dashboard.spec.ts"]
    );
}

#[test]
fn by_default_the_links_of_the_visited_pages_are_followed() {
    let (_d, root) = app();
    let pages = [
        "e2e/blog.spec.ts",
        "e2e/dashboard.spec.ts",
        "e2e/home.spec.ts",
        "e2e/pricing.spec.ts",
    ];
    // The header of the root layout links to `/` and `/pricing`: a test on any page
    // can click there, so a component of the home page or of the pricing page
    // reaches every spec that visits a page.
    assert_eq!(selected_by_default(&root, &["components/hero.tsx"]), pages);
    assert_eq!(
        selected_by_default(&root, &["components/button.tsx"]),
        pages
    );
    // No page links to a post or to the dashboard's settings from outside.
    assert_eq!(
        selected_by_default(&root, &["components/post-body.tsx"]),
        ["e2e/blog.spec.ts"]
    );
    assert_eq!(
        selected_by_default(&root, &["components/sidebar.tsx"]),
        ["e2e/dashboard.spec.ts"]
    );
    assert_eq!(
        selected_by_default(&root, &["lib/stats.ts"]),
        ["e2e/api.spec.ts", "e2e/dashboard.spec.ts"]
    );
    assert_eq!(
        selected_by_default(&root, &["README.md"]),
        Vec::<String>::new()
    );
    change(&root, "components/hero.tsx");
    let out = ok(
        &root,
        &[
            "affected",
            "--tests",
            "playwright",
            "--since",
            "HEAD",
            "--explain",
            "e2e/blog.spec.ts",
        ],
    );
    println!("{out}");
    assert!(
        out.starts_with("run e2e/blog.spec.ts\n  components/hero.tsx changed\n"),
        "{out}"
    );
    assert!(out.contains("→ an entry file of /"), "{out}");
    assert!(
        out.contains("/blog/[slug] links to it (components/site-header.tsx:"),
        "{out}"
    );
    assert!(
        out.contains("→ visited at e2e/blog.spec.ts:6 (page.goto /blog/hello-world)"),
        "{out}"
    );
}

#[test]
fn an_unrelated_change_selects_nothing() {
    let (_d, root) = app();
    assert_eq!(selected(&root, &["README.md"]), Vec::<String>::new());
    assert_eq!(
        selected(&root, &["zz-not-a-file.txt"]),
        Vec::<String>::new()
    );
    // A comment in a component is no change of the code.
    edit(
        &root,
        "components/hero.tsx",
        "export function",
        "// The headline.\nexport function",
    );
    assert_eq!(
        ok(
            &root,
            &["affected", "--tests", "playwright", "--since", "HEAD"]
        ),
        ""
    );
    let o = run(
        &root,
        &[
            "affected",
            "--tests",
            "playwright",
            "--since",
            "HEAD",
            "--exit-code",
        ],
    );
    assert_eq!(o.status.code(), Some(0));
    change(&root, "components/hero.tsx");
    let o = run(
        &root,
        &[
            "affected",
            "--tests",
            "playwright",
            "--since",
            "HEAD",
            "--exit-code",
        ],
    );
    assert_eq!(o.status.code(), Some(1));
}

#[test]
fn a_dynamic_import_and_a_server_action_are_followed() {
    let (_d, root) = app();
    // `next/dynamic(() => import(...))` on the pricing page.
    assert_eq!(
        selected(&root, &["components/plan-calculator.tsx"]),
        ["e2e/pricing.spec.ts"]
    );
    // `'use server'` action imported by a client component of the settings page,
    // and what the action imports.
    assert_eq!(
        selected(&root, &["app/actions.ts"]),
        ["e2e/dashboard.spec.ts"]
    );
    assert_eq!(
        selected(&root, &["lib/format.ts"]),
        ["e2e/dashboard.spec.ts"]
    );
}

#[test]
fn the_root_layout_reaches_every_page_and_no_handler() {
    let (_d, root) = app();
    let pages = [
        "e2e/blog.spec.ts",
        "e2e/dashboard.spec.ts",
        "e2e/home.spec.ts",
        "e2e/pricing.spec.ts",
    ];
    assert_eq!(selected(&root, &["components/site-header.tsx"]), pages);
    assert_eq!(selected(&root, &["app/layout.tsx"]), pages);
}

#[test]
fn a_handler_selects_the_specs_that_request_it_and_the_pages_that_fetch_it() {
    let (_d, root) = app();
    // `request.get('/api/stats')` in the spec; `fetch('/api/stats')` in a component
    // of the dashboard page.
    assert_eq!(
        selected(&root, &["lib/stats.ts"]),
        ["e2e/api.spec.ts", "e2e/dashboard.spec.ts"]
    );
}

#[test]
fn a_redirect_of_the_config_leads_to_its_destination() {
    let (_d, root) = app();
    // `/plans` redirects to `/pricing`: the spec that only names `/plans` in its
    // second test is the pricing spec; the table belongs to the destination.
    assert_eq!(
        selected(&root, &["components/pricing-table.tsx"]),
        ["e2e/pricing.spec.ts"]
    );
    // The config decides every route.
    assert_eq!(selected(&root, &["next.config.mjs"]), ALL);
}

#[test]
fn the_specs_own_code_selects_it() {
    let (_d, root) = app();
    assert_eq!(
        selected(&root, &["e2e/pages/dashboard-page.ts"]),
        ["e2e/dashboard.spec.ts"]
    );
    assert_eq!(selected(&root, &["e2e/home.spec.ts"]), ["e2e/home.spec.ts"]);
    assert_eq!(selected(&root, &["playwright.config.ts"]), ALL);
    // The server reads environment files when it starts.
    let out = ok(
        &root,
        &[
            "affected",
            "--tests",
            "playwright",
            "--changed",
            ".env.local",
        ],
    );
    assert_eq!(out.lines().collect::<Vec<_>>(), ALL);
}

#[test]
fn a_barrel_carries_a_change_only_to_the_routes_that_use_the_name() {
    let (_d, root) = app();
    // `components/index.ts` exports `Badge`, `Hero` and `PostBody`; the home page
    // imports `Hero` from it and the blog page `PostBody`. No route uses
    // `Badge`.
    assert_eq!(
        selected(&root, &["components/badge.tsx"]),
        Vec::<String>::new()
    );
    assert_eq!(
        selected(&root, &["components/hero.tsx"]),
        ["e2e/home.spec.ts"]
    );
    assert_eq!(
        selected(&root, &["components/post-body.tsx"]),
        ["e2e/blog.spec.ts"]
    );
    // Without a base revision the contents cannot be compared: the file-level edge
    // through the barrel counts for both pages.
    let out = ok(
        &root,
        &[
            "affected",
            "--tests",
            "playwright",
            "--changed",
            "components/badge.tsx",
            "--follow",
            "calls",
        ],
    );
    assert_eq!(out, "e2e/blog.spec.ts\ne2e/home.spec.ts\n");
}

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

fn commit(root: &Path) {
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "next"]);
}

const HANDLERS: &str = "import { readStats } from './stats';\n\nexport async function GET() {\n  \
                        return Response.json(readStats());\n}\n";

#[test]
fn a_route_file_that_only_reexports_runs_what_it_reexports() {
    // `app/api/stats/route.ts` is `export { GET } from '../../../lib/handlers'`:
    // the framework loads the route file and calls the handler it exports. The
    // route file is an entry, not a barrel on the way to an importer.
    let stats = ["e2e/api.spec.ts", "e2e/dashboard.spec.ts"];
    for route in [
        "export { GET } from '../../../lib/handlers';\n",
        "export * from '../../../lib/handlers';\n",
        "export { GET as GET, GET as HEAD } from '../../../lib/handlers';\n",
    ] {
        let (_d, root) = app();
        write(&root, "lib/handlers.ts", HANDLERS);
        write(&root, "app/api/stats/route.ts", route);
        commit(&root);
        assert_eq!(selected(&root, &["lib/handlers.ts"]), stats, "{route}");
        // What the handler calls is reached as before.
        assert_eq!(selected(&root, &["lib/stats.ts"]), stats, "{route}");
        assert_eq!(
            selected(&root, &["components/badge.tsx"]),
            Vec::<String>::new()
        );
    }
    // Behind a barrel the route file forwards one name: the file that provides it
    // selects the specs, another file of the barrel does not.
    let (_d, root) = app();
    write(&root, "lib/handlers.ts", HANDLERS);
    write(
        &root,
        "lib/limits.ts",
        "export function limit() {\n  return 10;\n}\n",
    );
    write(
        &root,
        "lib/index.ts",
        "export * from './handlers';\nexport * from './limits';\n",
    );
    write(
        &root,
        "app/api/stats/route.ts",
        "export { GET } from '../../../lib';\n",
    );
    commit(&root);
    assert_eq!(selected(&root, &["lib/handlers.ts"]), stats);
    assert_eq!(selected(&root, &["lib/limits.ts"]), Vec::<String>::new());
    // A page whose default export is another module's.
    let (_d, root) = app();
    let page = std::fs::read_to_string(root.join("app/pricing/page.tsx")).unwrap();
    write(
        &root,
        "components/pricing-page.tsx",
        &page.replace("../../components/", "./"),
    );
    write(
        &root,
        "app/pricing/page.tsx",
        "export { default } from '../../components/pricing-page';\n",
    );
    commit(&root);
    assert_eq!(
        selected(&root, &["components/pricing-page.tsx"]),
        ["e2e/pricing.spec.ts"]
    );
}

#[test]
fn a_route_files_own_code_selects_the_specs_of_its_route() {
    // A route file with nothing but exported functions has no side effects when it
    // is loaded and no module imports it: the framework is what calls its
    // exports.
    let (_d, root) = app();
    assert_eq!(
        selected(&root, &["app/api/stats/route.ts"]),
        ["e2e/api.spec.ts", "e2e/dashboard.spec.ts"]
    );
    assert_eq!(
        selected(&root, &["app/pricing/page.tsx"]),
        ["e2e/pricing.spec.ts"]
    );
}

#[test]
fn explain_prints_the_chain_from_the_component_to_the_specs_line() {
    let (_d, root) = app();
    change(&root, "components/button.tsx");
    let out = ok(
        &root,
        &[
            "affected",
            "--tests",
            "playwright",
            "--since",
            "HEAD",
            "--follow",
            "calls",
            "--explain",
            "e2e/dashboard.spec.ts",
        ],
    );
    println!("{out}");
    assert_eq!(
        out,
        "run e2e/dashboard.spec.ts
  components/button.tsx changed
    → imported by components/settings-form.tsx
    → imported by app/dashboard/settings/page.tsx
    → an entry file of /dashboard/settings
    → visited at e2e/dashboard.spec.ts:12 (a page object goes to /dashboard/settings)
  its source names 2 route(s): /dashboard (e2e/dashboard.spec.ts:6), /dashboard/settings \
         (e2e/dashboard.spec.ts:12)
  1 more through what those request: /api/stats
  assumed (--follow calls): it stays on these pages; their redirects and links are not followed
"
    );
    let out = ok(
        &root,
        &[
            "affected",
            "--tests",
            "playwright",
            "--since",
            "HEAD",
            "--follow",
            "calls",
            "--explain",
            "e2e/home.spec.ts",
        ],
    );
    println!("{out}");
    assert!(out.starts_with("skip e2e/home.spec.ts\n"), "{out}");
    assert!(
        out.contains("its source names 1 route(s): / (e2e/home.spec.ts:4)"),
        "{out}"
    );
    assert!(
        out.contains(".: the change reaches 2 of 7 route(s)"),
        "{out}"
    );
}

#[test]
fn json_lists_the_routes_the_change_reaches_and_each_specs_routes() {
    let (_d, root) = app();
    change(&root, "components/post-body.tsx");
    let out = ok(
        &root,
        &[
            "affected",
            "--tests",
            "playwright",
            "--since",
            "HEAD",
            "--follow",
            "calls",
            "--format",
            "json",
        ],
    );
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["mode"], "source");
    assert_eq!(v["specs"], 5);
    assert_eq!(v["selected"], 1);
    let app = &v["apps"][0];
    assert_eq!(app["routes"], 7);
    assert_eq!(app["reached"], 1);
    assert_eq!(app["reached_routes"][0]["route"], "/blog/[slug]");
    assert_eq!(
        app["reached_routes"][0]["chain"],
        serde_json::json!([
            "components/post-body.tsx",
            "components/index.ts",
            "app/blog/[slug]/page.tsx"
        ])
    );
    let blog = v["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["spec"] == "e2e/blog.spec.ts")
        .unwrap();
    assert_eq!(blog["run"], true);
    assert_eq!(blog["reasons"][0]["kind"], "route");
    assert_eq!(blog["reasons"][0]["route"], "/blog/[slug]");
    assert_eq!(
        blog["static"]["routes"],
        serde_json::json!(["/blog/[slug]"])
    );
    assert_eq!(blog["static"]["navigations"]["resolved"], 1);
}

#[test]
fn a_spec_with_an_unknown_url_may_visit_any_route() {
    let (_d, root) = app();
    std::fs::write(
        root.join("e2e/anywhere.spec.ts"),
        "import { test } from '@playwright/test';\n\ntest('goes where it is told', async ({ page \
         }) => {\n  await page.goto(process.env.START_URL!);\n});\n",
    )
    .unwrap();
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "a spec that goes anywhere"]);
    // Any route the change reaches selects it; a change that reaches none does not.
    assert_eq!(
        selected(&root, &["components/post-body.tsx"]),
        ["e2e/anywhere.spec.ts", "e2e/blog.spec.ts"]
    );
    assert_eq!(selected(&root, &["README.md"]), Vec::<String>::new());
    change(&root, "components/post-body.tsx");
    let out = ok(
        &root,
        &[
            "affected",
            "--tests",
            "playwright",
            "--since",
            "HEAD",
            "--follow",
            "calls",
            "--explain",
            "e2e/anywhere.spec.ts",
        ],
    );
    println!("{out}");
    assert!(out.contains("it may visit any route"), "{out}");
    assert!(out.contains("e2e/anywhere.spec.ts:4"), "{out}");
}

#[test]
fn a_fixture_value_names_data_and_an_unbound_argument_can_be_any_segment() {
    let (_d, root) = app();
    // `slug` is a fixture of the test callback: a name of data. It matches the
    // dynamic route, not the static sibling `/blog/archive`.
    std::fs::write(
        root.join("e2e/post.spec.ts"),
        "import { test as base } from '@playwright/test';\n\nconst test = base.extend<{ slug: \
         string }>({ slug: async ({}, use) => { await use(String(Date.now())); } });\n\ntest('a \
         new post', async ({ page, slug }) => {\n  await page.goto(`/blog/${slug}`);\n});\n",
    )
    .unwrap();
    // `name` is a parameter nothing binds here: it can be any string, `archive`
    // included.
    std::fs::write(
        root.join("e2e/section.spec.ts"),
        "import { test, type Page } from '@playwright/test';\n\nexport async function open(page: \
         Page, name: string) {\n  await page.goto(`/blog/${name}`);\n}\n\ntest('a section', async \
         ({ page }) => {\n  await open(page, process.argv[2]);\n});\n",
    )
    .unwrap();
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "two more specs"]);
    assert_eq!(
        selected(&root, &["components/archive-list.tsx"]),
        ["e2e/section.spec.ts"]
    );
    assert_eq!(
        selected(&root, &["components/post-body.tsx"]),
        [
            "e2e/blog.spec.ts",
            "e2e/post.spec.ts",
            "e2e/section.spec.ts"
        ]
    );
}

#[test]
fn summary_shows_the_flow_from_the_changed_file_to_the_specs() {
    let (_d, root) = app();
    change(&root, "components/button.tsx");
    let out = ok(
        &root,
        &[
            "affected",
            "--tests",
            "playwright",
            "--since",
            "HEAD",
            "--format",
            "summary",
        ],
    );
    println!("{out}");
    assert!(
        out.starts_with("4 of 5 spec(s) selected (80.0%), from source"),
        "{out}"
    );
    assert!(
        out.contains("      2       4  components/button.tsx"),
        "{out}"
    );
    assert!(out.contains(".: 2 of 7 route(s)"), "{out}");
    assert!(
        out.contains("a route it visits is reached by the change"),
        "{out}"
    );
    assert!(
        out.contains("5 spec(s) visit a known set of routes; 0 may visit any route"),
        "{out}"
    );
}

#[test]
fn summary_tells_a_changed_file_from_code_that_is_in_no_diff() {
    let (_d, root) = app();
    // A reader whose path is a parameter, and code loaded with it that has the
    // strings `/seed-data/` and `/content-data/`: the Playwright config reads a
    // seed file through it, the blog page a content file. The pricing page
    // loads a module by a computed name.
    write(
        &root,
        "lib/read.ts",
        "import fs from 'node:fs';\n\nexport function readText(p: string) {\n  return \
         fs.readFileSync(p, 'utf8');\n}\n",
    );
    write(
        &root,
        "e2e/support/paths.ts",
        "const base = process.env.DATA_ROOT;\n\nexport function seedPath(name: string) {\n  \
         return base + '/seed-data/' + name + '.json';\n}\n",
    );
    write(
        &root,
        "lib/paths.ts",
        "const base = process.env.DATA_ROOT;\n\nexport function contentPath(name: string) {\n  \
         return base + '/content-data/' + name + '.json';\n}\n",
    );
    let config = std::fs::read_to_string(root.join("playwright.config.ts")).unwrap();
    write(
        &root,
        "playwright.config.ts",
        &format!(
            "import {{ readText }} from './lib/read';\nimport {{ seedPath }} from \
             './e2e/support/paths';\n{config}\nexport const users = readText(seedPath('users'));\n"
        ),
    );
    let page = std::fs::read_to_string(root.join("app/blog/[slug]/page.tsx")).unwrap();
    write(
        &root,
        "app/blog/[slug]/page.tsx",
        &format!(
            "import {{ readText }} from '../../../lib/read';\nimport {{ contentPath }} from \
             '../../../lib/paths';\n{page}\nexport const posts = readText(contentPath('posts'));\n"
        ),
    );
    write(&root, "e2e/seed-data/users.json", "[]\n");
    write(&root, "content-data/posts.json", "[]\n");
    write(
        &root,
        "lib/plugin.ts",
        "export function plugin() {\n  return require(process.env.PLUGIN!);\n}\n",
    );
    edit(
        &root,
        "app/pricing/page.tsx",
        "import dynamic from 'next/dynamic';\n",
        "import dynamic from 'next/dynamic';\nimport { plugin } from \
         '../../lib/plugin';\n\nexport const extension = plugin();\n",
    );
    commit(&root);
    let run = |format: &str| {
        let base = ["affected", "--tests", "playwright", "--since", "HEAD"];
        let mut args = base.to_vec();
        args.extend(["--follow", "calls", "--format", format]);
        ok(&root, &args)
    };
    let sections = |out: &str| -> (String, String) {
        let (_, rest) = out
            .split_once(
                "\nFrom where: what changed at the start of the chain of each route and spec\n",
            )
            .unwrap_or_else(|| panic!("{out}"));
        let (changed, rest) = rest
            .split_once("\nThrough code whose dependencies are not known: where that code is")
            .unwrap_or_else(|| panic!("{out}"));
        let (not_changed, _) = rest
            .split_once("\nThrough what:")
            .unwrap_or_else(|| panic!("{out}"));
        (changed.to_string(), not_changed.to_string())
    };

    // 1. A change that reaches nothing: the pricing spec runs because of the load
    //    with an
    // unknown target. The file with the load is in no diff and is not listed as
    // changed.
    write(&root, "README.md", "changed\n");
    let out = run("summary");
    println!("{out}");
    assert!(out.starts_with("1 of 5 spec(s) selected"), "{out}");
    let (changed, not_changed) = sections(&out);
    assert!(changed.contains("no changed file reaches a route"), "{out}");
    assert!(
        not_changed.contains("      1       1  lib/plugin.ts (selected on every change)"),
        "{out}"
    );
    assert!(
        out.contains("← lib/plugin.ts (not changed: its dependencies are not known)"),
        "{out}"
    );
    assert!(
        out.contains("a route it visits reaches code with unknown dependencies"),
        "{out}"
    );
    git(&root, &["checkout", "-q", "."]);

    // 2. A file that the code of a route names: the changed file starts the chain,
    //    not
    // the file with the string.
    write(&root, "content-data/posts.json", "[1]\n");
    let out = run("summary");
    println!("{out}");
    assert!(out.starts_with("2 of 5 spec(s) selected"), "{out}");
    let (changed, _) = sections(&out);
    assert!(
        changed.contains("      1       1  content-data/posts.json"),
        "{out}"
    );
    assert!(!changed.contains("lib/"), "{out}");
    assert!(
        out.contains("/blog/[slug]  ← app/blog/[slug]/page.tsx ← content-data/posts.json"),
        "{out}"
    );
    let v: Value = serde_json::from_str(&run("json")).unwrap();
    let blog = v["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["spec"] == "e2e/blog.spec.ts")
        .unwrap();
    assert_eq!(blog["reasons"][0]["cause"], "named", "{blog}");
    assert_eq!(
        blog["reasons"][0]["changed"], "content-data/posts.json",
        "{blog}"
    );
    git(&root, &["checkout", "-q", "."]);

    // 3. A file that the Playwright config names: the config did not change.
    write(&root, "e2e/seed-data/users.json", "[1]\n");
    let out = run("summary");
    println!("{out}");
    assert!(out.starts_with("5 of 5 spec(s) selected"), "{out}");
    assert!(
        out.contains(
            "5 100.0%  a file changed that its Playwright setup names, lists or looks for"
        ),
        "{out}"
    );
    assert!(!out.contains("its Playwright setup changed"), "{out}");
    let v: Value = serde_json::from_str(&run("json")).unwrap();
    for r in v["results"].as_array().unwrap() {
        assert_eq!(r["reasons"][0]["kind"], "playwright_config", "{r}");
        assert_eq!(r["reasons"][0]["cause"], "named", "{r}");
    }
    git(&root, &["checkout", "-q", "."]);

    // 4. The config itself, and a setup that reaches unknown dependencies.
    edit(&root, "playwright.config.ts", "./e2e'", "./e2e/'");
    let out = run("summary");
    assert!(
        out.contains(
            "5 100.0%  its Playwright setup changed (the config, the global setup or what they \
             load)"
        ),
        "{out}"
    );
    git(&root, &["checkout", "-q", "."]);
    edit(
        &root,
        "playwright.config.ts",
        "export const users",
        "export const more = require(process.env.MORE!);\nexport const users",
    );
    commit(&root);
    write(&root, "README.md", "changed\n");
    let out = run("summary");
    assert!(
        out.contains("5 100.0%  its Playwright setup reaches unknown dependencies"),
        "{out}"
    );
}

#[test]
fn other_kinds_of_change_run_every_spec_of_the_app() {
    let (_d, root) = app();
    // A file served from `public/` is requested by URL: the source does not say by
    // whom.
    std::fs::create_dir_all(root.join("public")).unwrap();
    std::fs::write(root.join("public/logo.svg"), "<svg/>").unwrap();
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "a public file"]);
    assert_eq!(selected(&root, &["public/logo.svg"]), ALL);
    // A removed input of every route.
    std::fs::remove_file(root.join("next.config.mjs")).unwrap();
    let out = ok(
        &root,
        &["affected", "--tests", "playwright", "--since", "HEAD"],
    );
    assert_eq!(out.lines().collect::<Vec<_>>(), ALL);
    git(&root, &["checkout", "-q", "."]);
    // A removed page: the route table of the tree no longer has it.
    std::fs::remove_file(root.join("app/blog/archive/page.tsx")).unwrap();
    let out = ok(
        &root,
        &["affected", "--tests", "playwright", "--since", "HEAD"],
    );
    assert_eq!(out.lines().collect::<Vec<_>>(), ALL);
}

#[test]
fn a_helper_behind_index_files_is_read_and_a_url_without_a_value_is_unknown() {
    let (_d, root) = app();
    // The navigation is three modules away from the spec, behind two index files.
    let write = |rel: &str, text: &str| {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    };
    write("e2e/support/index.ts", "export * from './nav';\n");
    write(
        "e2e/support/nav/index.ts",
        "export { openPricing } from './open';\n",
    );
    write(
        "e2e/support/nav/open.ts",
        "import type { Page } from '@playwright/test';\n\nexport async function openPricing(page: \
         Page) {\n  await page.goto('/pricing');\n}\n",
    );
    write(
        "e2e/deep.spec.ts",
        "import { test } from '@playwright/test';\nimport { openPricing } from \
         './support';\n\ntest('pricing through a helper', async ({ page }) => {\n  await \
         openPricing(page);\n});\n",
    );
    // A URL that a hook assigns: the evaluator has no value for it.
    write(
        "e2e/later.spec.ts",
        "import { test } from '@playwright/test';\n\nlet inviteUrl: \
         string;\n\ntest.beforeAll(async () => {\n  ({ inviteUrl } = await (globalThis as \
         any).makeInvite());\n});\n\ntest('goes to the invite', async ({ page }) => {\n  await \
         page.goto(inviteUrl);\n});\n",
    );
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "helpers"]);
    assert_eq!(
        selected(&root, &["components/plan-calculator.tsx"]),
        [
            "e2e/deep.spec.ts",
            "e2e/later.spec.ts",
            "e2e/pricing.spec.ts"
        ]
    );
    change(&root, "components/plan-calculator.tsx");
    let out = ok(
        &root,
        &[
            "affected",
            "--tests",
            "playwright",
            "--since",
            "HEAD",
            "--follow",
            "calls",
            "--explain",
            "e2e/later.spec.ts",
        ],
    );
    assert!(out.contains("it may visit any route"), "{out}");
    assert!(out.contains("e2e/later.spec.ts:10"), "{out}");
}

/// Pages under `/[locale]` behind a middleware that rewrites `/x` to
/// `/<locale>/x`: the code names its pages without the locale, in a spec's
/// `goto` and in a `redirect()` alike, and both are matched behind the segment
/// the middleware puts in front.
#[test]
fn a_redirect_in_a_loader_leads_to_the_page_behind_the_middlewares_prefix() {
    let (_d, root) = app();
    let write = |rel: &str, text: &str| {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    };
    write(
        "middleware.ts",
        "import { NextResponse, type NextRequest } from 'next/server';\n\nexport function \
         middleware(req: NextRequest) {\n  const url = req.nextUrl.clone();\n  url.pathname = \
         `/${req.cookies.get('locale')?.value ?? 'en'}${url.pathname}`;\n  return \
         NextResponse.rewrite(url);\n}\n\nexport const config = { matcher: ['/account', \
         '/welcome'] };\n",
    );
    // The page's layout loads the member; the loader sends a visitor to `/welcome`.
    write(
        "lib/member.ts",
        "import { redirect } from 'next/navigation';\nimport { cookies } from \
         'next/headers';\n\nexport async function loadMember() {\n  const name = (await \
         cookies()).get('member')?.value;\n  if (!name) {\n    redirect('/welcome');\n  }\n  \
         return { name };\n}\n",
    );
    write(
        "app/[locale]/account/layout.tsx",
        "import { loadMember } from '../../../lib/member';\n\nexport default async function \
         AccountLayout({ children }: { children: React.ReactNode }) {\n  const member = await \
         loadMember();\n  return <section title={member.name}>{children}</section>;\n}\n",
    );
    write(
        "app/[locale]/account/page.tsx",
        "export default function Account() {\n  return <main>Account</main>;\n}\n",
    );
    write(
        "app/[locale]/welcome/page.tsx",
        "import { WelcomeCard } from '../../../components/welcome-card';\n\nexport default \
         function Welcome() {\n  return <WelcomeCard />;\n}\n",
    );
    write(
        "components/welcome-card.tsx",
        "export function WelcomeCard() {\n  return <p>Welcome</p>;\n}\n",
    );
    write(
        "e2e/account.spec.ts",
        "import { test, expect } from '@playwright/test';\n\ntest('a visitor is sent to the \
         welcome page', async ({ page }) => {\n  await page.goto('/account');\n  await \
         expect(page.getByText('Welcome')).toBeVisible();\n});\n",
    );
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "locale"]);
    // The spec names `/account` only. Its page is `/[locale]/account`, whose
    // layout's loader redirects to `/welcome`: the page `/[locale]/welcome`.
    assert_eq!(
        selected_with(
            &root,
            &["components/welcome-card.tsx"],
            &["--follow", "redirects"]
        ),
        ["e2e/account.spec.ts"]
    );
    // Not followed when a spec is assumed to stay on the pages it names.
    assert_eq!(
        selected(&root, &["components/welcome-card.tsx"]),
        Vec::<String>::new()
    );
    assert!(
        selected_by_default(&root, &["components/welcome-card.tsx"])
            .contains(&"e2e/account.spec.ts".to_string())
    );
    change(&root, "components/welcome-card.tsx");
    let out = ok(
        &root,
        &[
            "affected",
            "--tests",
            "playwright",
            "--since",
            "HEAD",
            "--follow",
            "redirects",
            "--explain",
            "e2e/account.spec.ts",
        ],
    );
    println!("{out}");
    assert!(
        out.contains("→ an entry file of /[locale]/welcome"),
        "{out}"
    );
    assert!(out.contains("(lib/member.ts:7)"), "{out}");
    assert!(
        out.contains("→ visited at e2e/account.spec.ts:4 (page.goto /account"),
        "{out}"
    );
}

/// An app's own link component hands its props to `next/link`, so its file
/// names no URL: the `href` where the component is used is where the link goes.
#[test]
fn a_link_through_the_apps_own_link_component_is_followed() {
    let (_d, root) = app();
    // Nobody links to the archive and no spec names it.
    assert_eq!(
        selected_by_default(&root, &["components/archive-list.tsx"]),
        Vec::<String>::new()
    );
    std::fs::write(
        root.join("components/app-link.tsx"),
        "import NextLink from 'next/link';\nimport type { ComponentProps } from \
         'react';\n\nexport function AppLink({ prefetch = false, ...props }: \
         ComponentProps<typeof NextLink>) {\n  return <NextLink prefetch={prefetch} {...props} \
         />;\n}\n",
    )
    .unwrap();
    edit(
        &root,
        "components/sidebar.tsx",
        "import Link from 'next/link';",
        "import Link from 'next/link';\nimport { AppLink } from './app-link';",
    );
    edit(
        &root,
        "components/sidebar.tsx",
        "<Link href=\"/dashboard/settings\">Settings</Link>",
        "<Link href=\"/dashboard/settings\">Settings</Link>\n      <AppLink \
         href=\"/blog/archive\">Archive</AppLink>",
    );
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "a link to the archive"]);
    // The dashboard's sidebar links there: a test on the dashboard can click it.
    assert_eq!(
        selected_by_default(&root, &["components/archive-list.tsx"]),
        ["e2e/dashboard.spec.ts"]
    );
    // A spec that is assumed to stay on the pages it names does not follow the
    // link.
    assert_eq!(
        selected(&root, &["components/archive-list.tsx"]),
        Vec::<String>::new()
    );
}

#[test]
fn recordings_without_a_build_are_refused() {
    let (_d, root) = app();
    let o = run(
        &root,
        &[
            "affected",
            "--tests",
            "playwright",
            "--changed",
            "README.md",
            "--visits",
            "v",
        ],
    );
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("--next-fingerprint"));
}

/// `affected --tests playwright --format json` for the working tree against the
/// base commit, without its timings. `env`: more environment.
fn json_of(root: &Path, more: &[&str], env: &[(&str, &str)]) -> Value {
    let mut c = Command::new(support::harness());
    c.args([
        "affected",
        "--tests",
        "playwright",
        "--since",
        "HEAD",
        "--format",
        "json",
    ])
    .args(more)
    .current_dir(root)
    .env_remove("TURBO_TRACE_DEBUG_E2E_FOLLOW");
    for (k, v) in env {
        c.env(k, v);
    }
    let o = c.output().expect("run turbotrace");
    assert!(
        o.status.success(),
        "turbotrace affected failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    let mut v: Value = serde_json::from_slice(&o.stdout).unwrap();
    v.as_object_mut().unwrap().remove("timings_ms");
    v
}

/// The facts kept between queries (what a spec visits, the relation of an app,
/// what following adds: `<graph dir>/cache/e2e-*`) change how long a query
/// takes and never its answer: after every edit of a sequence, under every
/// rule, the answer with the stored facts is the answer computed from nothing.
#[test]
fn stored_facts_give_the_answer_computed_from_scratch() {
    let (_d, root) = app();
    const SCRATCH: &[(&str, &str)] = &[("TURBO_TRACE_DEBUG_E2E_CACHE", "0")];
    let rules: [&[&str]; 4] = [
        &[],
        &["--follow", "redirects"],
        &["--follow", "calls"],
        &["--follow", "named"],
    ];
    let mut step = 0;
    let mut same = |what: &str| {
        step += 1;
        for rule in rules {
            // Twice with the store (the first query may fill it, the second reads it),
            // once without.
            let first = json_of(&root, rule, &[]);
            let second = json_of(&root, rule, &[]);
            let scratch = json_of(&root, rule, SCRATCH);
            assert_eq!(
                first, scratch,
                "step {step} ({what}), {rule:?}: stored against scratch"
            );
            assert_eq!(
                second, scratch,
                "step {step} ({what}), {rule:?}: read back against scratch"
            );
        }
    };
    // A component of one page: the facts are stored by the first query.
    change(&root, "components/hero.tsx");
    same("a component changed");
    assert!(
        root.join(".turbo/trace/cache/e2e-specs").is_file(),
        "no stored facts in {:?}",
        std::fs::read_dir(root.join(".turbo/trace/cache")).map(|d| d
            .filter_map(|e| e.ok().map(|e| e.file_name()))
            .collect::<Vec<_>>())
    );
    // The shared header links to another page: the relation of the app changes, the
    // visits of the specs do not.
    edit(
        &root,
        "components/site-header.tsx",
        "<Link href=\"/pricing\">Pricing</Link>",
        "<Link href=\"/pricing\">Pricing</Link>\n      <Link href=\"/blog/archive\">Archive</Link>",
    );
    same("a link added to the header");
    // A page object goes somewhere else: the visits of the specs that use it
    // change.
    edit(
        &root,
        "e2e/pages/dashboard-page.ts",
        "await this.page.goto('/dashboard/settings');",
        "await this.page.goto('/pricing');",
    );
    same("a page object visits another page");
    // A spec gets a URL that cannot be evaluated: it may visit any route.
    edit(
        &root,
        "e2e/home.spec.ts",
        "await page.goto('/');",
        "await page.goto(process.env.START_URL ?? '/');",
    );
    same("a URL of a spec is not known");
    // A route is added and one is removed: the route table changes (every key
    // does).
    std::fs::create_dir_all(root.join("app/about")).unwrap();
    std::fs::write(
        root.join("app/about/page.tsx"),
        "import Link from 'next/link';\n\nexport default function About() {\n  return <Link \
         href=\"/dashboard\">Dashboard</Link>;\n}\n",
    )
    .unwrap();
    std::fs::remove_file(root.join("app/blog/archive/page.tsx")).unwrap();
    same("a route added, a route removed");
    // A redirect in the code of a page, with a target that is not known.
    edit(
        &root,
        "app/pricing/page.tsx",
        "export default",
        "import { redirect } from 'next/navigation';\n\nexport function leave(to: string) {\n  \
         redirect(to);\n}\n\nexport default",
    );
    same("a redirect with an unknown target");
    // Everything back: the facts of the first state are no longer the stored ones.
    git(&root, &["checkout", "-q", "."]);
    git(&root, &["clean", "-q", "-fd", "-e", ".turbo"]);
    change(&root, "components/pricing-table.tsx");
    same("back at the base, another component changed");
}

/// `TURBO_TRACE_DEBUG_E2E_CHECK=1`: what is kept per spec and app equals one
/// walk over the relations of one scan of every app's code.
#[test]
fn the_reach_kept_per_app_is_the_reach_of_one_walk() {
    let (_d, root) = app();
    change(&root, "components/hero.tsx");
    for rule in ["links", "redirects", "calls"] {
        let o = Command::new(support::harness())
            .args([
                "affected",
                "--tests",
                "playwright",
                "--since",
                "HEAD",
                "--follow",
                rule,
            ])
            .current_dir(&root)
            .env_remove("TURBO_TRACE_DEBUG_E2E_FOLLOW")
            .env("TURBO_TRACE_DEBUG_E2E_CHECK", "1")
            .env("TURBO_TRACE_DEBUG_E2E_ALL_RULES", "1")
            .output()
            .expect("run turbotrace");
        let err = String::from_utf8_lossy(&o.stderr);
        assert!(o.status.success(), "{err}");
        assert!(
            err.contains("compared with one walk over one scan: 0 differ"),
            "{rule}: {err}"
        );
    }
}

/// The fixture app as `apps/web` of a pnpm workspace (a package resolves only
/// what it declares), with a page that loads a module by a value nothing
/// evaluates and a spec that visits it:
///
/// ```text
/// apps/web/app/plug/page.tsx     load(globalThis.plugin)              web declares @x/lib
/// apps/web/lib/load.ts           load(name): import(name)
/// apps/web/e2e/plug.spec.ts      goes to /plug
/// packages/lib/extra.js          nothing imports it
/// packages/other/index.js        nobody declares @x/other
/// ```
#[cfg(unix)]
fn workspace_app() -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../turborepo-trace-testkit/fixtures/next-e2e");
    let root = d.path().join("repo");
    copy(&fixture, &root.join("apps/web"));
    let root = std::fs::canonicalize(root).unwrap();
    let write = |rel: &str, text: &str| {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    };
    write("package.json", r#"{"name":"r","private":true}"#);
    write(
        "pnpm-workspace.yaml",
        "packages:\n  - 'apps/*'\n  - 'packages/*'\n",
    );
    write(".gitignore", ".turbo/trace\nnode_modules\n");
    let manifest = std::fs::read_to_string(root.join("apps/web/package.json")).unwrap();
    write(
        "apps/web/package.json",
        &manifest.replace(
            "\"dependencies\": {",
            "\"dependencies\": {\n    \"@x/lib\": \"workspace:*\",",
        ),
    );
    write(
        "packages/lib/package.json",
        r#"{"name":"@x/lib","version":"1.0.0"}"#,
    );
    write("packages/lib/index.js", "module.exports = 1;\n");
    write("packages/lib/extra.js", "module.exports = 2;\n");
    write(
        "packages/other/package.json",
        r#"{"name":"@x/other","version":"1.0.0"}"#,
    );
    write("packages/other/index.js", "module.exports = 3;\n");
    let link = root.join("apps/web/node_modules/@x/lib");
    std::fs::create_dir_all(link.parent().unwrap()).unwrap();
    turborepo_trace_core::os::symlink(root.join("packages/lib"), link).unwrap();
    write(
        "apps/web/lib/load.ts",
        "export function load(name: string) {\n  return import(name);\n}\n",
    );
    write(
        "apps/web/app/plug/page.tsx",
        "import { load } from '../../lib/load';\n\nexport default async function Page() {\n  \
         const plugin = await load((globalThis as { plugin?: string }).plugin ?? '');\n  return \
         <main>{String(plugin)}</main>;\n}\n",
    );
    write(
        "apps/web/e2e/plug.spec.ts",
        "import { test } from '@playwright/test';\n\ntest('the plugin page renders', async ({ \
         page }) => {\n  await page.goto('/plug');\n});\n",
    );
    git(&root, &["init", "-q"]);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "base"]);
    (d, root)
}

/// A route that reaches a module load with an unknown target is not selected on
/// every change: the declared packages of the app bound the load, as for every
/// other entry. The spec that visits the route is selected for a change in a
/// package the app depends on, and the reason says so.
#[cfg(unix)]
#[test]
fn a_route_with_a_load_of_unknown_target_is_selected_for_what_its_app_depends_on() {
    let (_d, root) = workspace_app();
    const PLUG: &str = "apps/web/e2e/plug.spec.ts";
    let run_env = |args: &[&str], env: &[(&str, &str)]| -> String {
        let mut c = Command::new(support::harness());
        c.args(args)
            .current_dir(&root)
            .env_remove("TURBO_TRACE_DEBUG_E2E_FOLLOW");
        for (k, v) in env {
            c.env(k, v);
        }
        let o = c.output().expect("run turbotrace");
        assert!(
            o.status.success(),
            "turbotrace {args:?} failed: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8(o.stdout).unwrap()
    };
    let specs = |env: &[(&str, &str)]| -> Vec<String> {
        run_env(
            &[
                "affected",
                "--tests",
                "playwright",
                "--since",
                "HEAD",
                "--follow",
                "calls",
            ],
            env,
        )
        .lines()
        .map(String::from)
        .collect()
    };
    const OPEN: &[(&str, &str)] = &[("TURBO_TRACE_DEBUG_LOADS_PACKAGE", "0")];
    // A package the app does not declare: no spec. Without the bound the load is
    // open and its route is reached by every change.
    change(&root, "packages/other/index.js");
    assert!(specs(&[]).is_empty(), "{:?}", specs(&[]));
    assert_eq!(specs(OPEN), [PLUG]);
    git(&root, &["checkout", "-q", "."]);
    // A file of a package the app declares, which nothing imports.
    change(&root, "packages/lib/extra.js");
    assert_eq!(specs(&[]), [PLUG]);
    let why = run_env(
        &[
            "affected",
            "--tests",
            "playwright",
            "--since",
            "HEAD",
            "--follow",
            "calls",
            "--explain",
            PLUG,
        ],
        &[],
    );
    println!("{why}");
    assert!(why.starts_with(&format!("run {PLUG}\n")), "{why}");
    assert!(
        why.contains(
            "is an entry file of /plug and reaches a module load whose target is not known ("
        ),
        "{why}"
    );
    assert!(
        why.contains("the load can reach packages/lib/ (packages/lib/extra.js changed)"),
        "{why}"
    );
    assert!(!why.contains("so any change may affect it"), "{why}");
    let summary = run_env(
        &[
            "affected",
            "--tests",
            "playwright",
            "--since",
            "HEAD",
            "--follow",
            "calls",
            "--format",
            "summary",
        ],
        &[],
    );
    println!("{summary}");
    assert!(
        summary.contains(
            "apps/web/app/plug/page.tsx (not changed: it has a load with an unknown target)"
        ),
        "{summary}"
    );
    assert!(
        summary.contains(
            "      1       1  apps/web/app/plug/page.tsx (a load with an unknown target: the \
             change affects a package its app depends on)"
        ),
        "{summary}"
    );
    assert!(
        summary.contains(
            "a route it visits has a load with an unknown target, and the change affects a \
             package its app depends on"
        ),
        "{summary}"
    );
    let v: Value = serde_json::from_str(&run_env(
        &[
            "affected",
            "--tests",
            "playwright",
            "--since",
            "HEAD",
            "--follow",
            "calls",
            "--format",
            "json",
        ],
        &[],
    ))
    .unwrap();
    let text = v.to_string();
    assert!(text.contains("the load can reach packages/lib/"), "{text}");
}
