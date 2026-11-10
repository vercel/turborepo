//! The validation tooling behind `turbotrace affected --tests playwright` (the
//! hidden flags `--next-fingerprint`, `--visits`, `--base-fingerprint`; the
//! oracle of the selection from source, which `affected_playwright_source.rs`
//! tests): end-to-end specs are decided by what they reached on the recorded
//! build and whether that is the same on the build under test, plus the
//! ordinary verdict of their own imports.
// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{
    path::Path,
    process::{Command, Output},
};

use serde_json::{Value, json};

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(support::harness())
        .args(args)
        .current_dir(root)
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

/// A fingerprint of a build with four routes; `edit` changes it.
fn fingerprint(build: &str, edit: impl FnOnce(&mut Value)) -> String {
    let route = |own: &str, kind: &str| {
        json!({
            "key": "k", "router": "app", "kind": kind,
            "server": {"hash": format!("s-{own}"), "files": 1},
            "client": {"hash": format!("c-{own}"), "files": 1},
            "css": {"hash": "x", "files": 0}, "prerender": {"hash": "p", "files": 0},
            "external": "e", "own": own, "effective": own
        })
    };
    let mut v = json!({
        "version": 3, "build_id": build, "bundler": "turbopack", "per_module": true,
        "routes": {
            "/": route("home", "page"),
            "/about": route("about", "page"),
            "/blog/[slug]": route("blog", "page"),
            "/api/hello": route("hello", "handler"),
            "/_not-found": route("nf", "page")
        },
        "global": {"routing": "r1", "route_table": "t1", "config": "c1", "images": "i1"},
        "rules": [],
        "route_regexes": [
            {"page": "/", "regex": "^/(?:/)?$"},
            {"page": "/_not-found", "regex": "^/_not\\-found(?:/)?$"},
            {"page": "/about", "regex": "^/about(?:/)?$"},
            {"page": "/api/hello", "regex": "^/api/hello(?:/)?$"},
            {"page": "/blog/[slug]", "regex": "^/blog/([^/]+?)(?:/)?$"}
        ],
        "public": {"logo.png": "h1"}
    });
    edit(&mut v);
    v.to_string()
}

fn recording(
    spec: &str,
    title: &str,
    build: Option<&str>,
    status: &str,
    requests: Value,
) -> String {
    json!({
        "schema": 1, "recorder": "0.1.0", "spec": spec, "specHash": null,
        "title": [title], "project": "chromium", "retry": 0,
        "status": status, "expectedStatus": "passed",
        "startedAt": "2026-01-01T00:00:00.000Z",
        "baseURL": "http://localhost:3000", "origins": ["http://localhost:3000"],
        "build": build, "commit": null, "requests": requests, "external": {}
    })
    .to_string()
}

fn get(kind: &str, path: &str) -> Value {
    json!({"kind": kind, "method": "GET", "path": path, "status": 200, "n": 1})
}

/// An app with three recorded specs (`about`, `blog`, `api`), one spec without
/// a recording (`fresh`) and one whose recorded run failed (`broken`).
fn repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"app","private":true}"#);
    write(r, ".gitignore", ".turbo/trace/\n");
    write(
        r,
        "playwright.config.ts",
        "export default { testDir: './e2e', use: { baseURL: 'http://localhost:3000' } };\n",
    );
    write(
        r,
        "e2e/helpers.ts",
        "export const login = async () => {};\n",
    );
    for (name, import) in [
        ("about", "import { login } from './helpers';\n"),
        ("blog", ""),
        ("api", ""),
        ("fresh", ""),
        ("broken", ""),
    ] {
        write(
            r,
            &format!("e2e/{name}.spec.ts"),
            &format!(
                "{import}import {{ test }} from '@playwright/test';\ntest('{name}', async () => \
                 {{}});\n"
            ),
        );
    }
    write(r, "visits/fingerprint.json", &fingerprint("b1", |_| {}));
    write(
        r,
        "visits/about.json",
        &recording(
            "e2e/about.spec.ts",
            "about",
            Some("b1"),
            "passed",
            json!([
                get("document", "/about"),
                get("fetch", "/logo.png"),
                get("fetch", "/api/hello")
            ]),
        ),
    );
    write(
        r,
        "visits/blog.json",
        &recording(
            "e2e/blog.spec.ts",
            "blog",
            Some("b1"),
            "passed",
            json!([
                get("document", "/"),
                get("nav", "/blog/first"),
                get("prefetch", "/about")
            ]),
        ),
    );
    write(
        r,
        "visits/api.json",
        &recording(
            "e2e/api.spec.ts",
            "api",
            Some("b1"),
            "passed",
            json!([{"kind": "api", "method": "POST", "path": "/api/hello", "status": 200, "n": 2}]),
        ),
    );
    write(
        r,
        "visits/broken.json",
        &recording(
            "e2e/broken.spec.ts",
            "broken",
            Some("b1"),
            "timedOut",
            json!([get("document", "/")]),
        ),
    );
    d
}

fn select(r: &Path, head: &str, changed: &str, extra: &[&str]) -> Vec<String> {
    write(r, "head.json", head);
    let mut args = vec![
        "affected",
        "--tests",
        "playwright",
        "--next-fingerprint",
        "head.json",
        "--visits",
        "visits",
        "--changed",
        changed,
    ];
    args.extend(extra);
    ok(r, &args).lines().map(str::to_string).collect()
}

fn explain(r: &Path, head: &str, changed: &str, spec: &str) -> String {
    write(r, "head.json", head);
    ok(
        r,
        &[
            "affected",
            "--tests",
            "playwright",
            "--next-fingerprint",
            "head.json",
            "--visits",
            "visits",
            "--changed",
            changed,
            "--explain",
            spec,
        ],
    )
}

const ALWAYS: [&str; 2] = ["e2e/broken.spec.ts", "e2e/fresh.spec.ts"];

#[test]
fn an_identical_build_runs_only_specs_without_a_usable_recording() {
    let d = repo();
    let r = d.path();
    assert_eq!(
        select(r, &fingerprint("b2", |_| {}), "README.md", &[]),
        ALWAYS
    );
    let text = explain(
        r,
        &fingerprint("b2", |_| {}),
        "README.md",
        "e2e/fresh.spec.ts",
    );
    assert!(
        text.starts_with("run e2e/fresh.spec.ts\n  no recording"),
        "{text}"
    );
    let text = explain(
        r,
        &fingerprint("b2", |_| {}),
        "README.md",
        "e2e/broken.spec.ts",
    );
    assert!(
        text.contains("the recorded run of \"broken\" timed out: its requests are incomplete"),
        "{text}"
    );
    let text = explain(
        r,
        &fingerprint("b2", |_| {}),
        "README.md",
        "e2e/blog.spec.ts",
    );
    assert!(text.starts_with("skip e2e/blog.spec.ts\n"), "{text}");
    assert!(
        text.contains("reaching 2 route(s): /, /blog/[slug]"),
        "{text}"
    );
}

#[test]
fn a_changed_route_runs_the_specs_that_reached_it() {
    let d = repo();
    let r = d.path();
    let head = fingerprint("b2", |v| {
        v["routes"]["/blog/[slug]"]["client"]["hash"] = "c-new".into();
        v["routes"]["/blog/[slug]"]["own"] = "new".into();
        v["routes"]["/blog/[slug]"]["effective"] = "new".into();
    });
    assert_eq!(
        select(r, &head, "README.md", &[]),
        [
            "e2e/blog.spec.ts",
            "e2e/broken.spec.ts",
            "e2e/fresh.spec.ts"
        ]
    );
    let text = explain(r, &head, "README.md", "e2e/blog.spec.ts");
    assert!(
        text.contains("visits /blog/[slug] (as /blog/first), whose client chunks changed"),
        "{text}"
    );
    // A handler: the spec that calls it and the page spec whose page fetches it.
    let head = fingerprint("b2", |v| {
        v["routes"]["/api/hello"]["server"]["hash"] = "s-new".into();
        v["routes"]["/api/hello"]["own"] = "new".into();
        v["routes"]["/api/hello"]["effective"] = "new".into();
    });
    assert_eq!(
        select(r, &head, "README.md", &[]),
        [
            "e2e/about.spec.ts",
            "e2e/api.spec.ts",
            "e2e/broken.spec.ts",
            "e2e/fresh.spec.ts"
        ]
    );
    let text = explain(r, &head, "README.md", "e2e/api.spec.ts");
    assert!(
        text.contains("sends POST to /api/hello, whose server code changed"),
        "{text}"
    );
    // A prefetch is not a visit: /about changes, the blog spec stays skipped.
    let head = fingerprint("b2", |v| {
        v["routes"]["/about"]["server"]["hash"] = "s-new".into();
        v["routes"]["/about"]["own"] = "new".into();
        v["routes"]["/about"]["effective"] = "new".into();
    });
    assert_eq!(
        select(r, &head, "README.md", &[]),
        [
            "e2e/about.spec.ts",
            "e2e/broken.spec.ts",
            "e2e/fresh.spec.ts"
        ]
    );
}

#[test]
fn imports_public_files_and_global_inputs() {
    let d = repo();
    let r = d.path();
    let same = fingerprint("b2", |_| {});
    // The spec's own imports: the ordinary verdict.
    assert_eq!(
        select(r, &same, "e2e/helpers.ts", &[]),
        [
            "e2e/about.spec.ts",
            "e2e/broken.spec.ts",
            "e2e/fresh.spec.ts"
        ]
    );
    let text = explain(r, &same, "e2e/helpers.ts", "e2e/about.spec.ts");
    assert!(
        text.contains("the spec imports e2e/helpers.ts, which changed"),
        "{text}"
    );
    // The Playwright config and environment files: every spec.
    assert_eq!(select(r, &same, "playwright.config.ts", &[]).len(), 5);
    write(r, ".env.test", "FLAG=1\n");
    let all = select(r, &same, ".env.test", &[]);
    assert_eq!(all.len(), 5, "{all:?}");
    let text = explain(r, &same, ".env.test", "e2e/blog.spec.ts");
    assert!(
        text.contains("the environment file .env.test changed"),
        "{text}"
    );
    // A requested public file.
    let head = fingerprint("b2", |v| v["public"]["logo.png"] = "h2".into());
    assert_eq!(
        select(r, &head, "README.md", &[]),
        [
            "e2e/about.spec.ts",
            "e2e/broken.spec.ts",
            "e2e/fresh.spec.ts"
        ]
    );
    // The evaluated config: every spec.
    let head = fingerprint("b2", |v| v["global"]["config"] = "c2".into());
    assert_eq!(select(r, &head, "README.md", &[]).len(), 5);
    let text = explain(r, &head, "README.md", "e2e/blog.spec.ts");
    assert!(text.contains("the Next.js config changed"), "{text}");
}

#[test]
fn recordings_need_a_known_build_and_the_recorded_spec() {
    let d = repo();
    let r = d.path();
    let same = fingerprint("b3", |_| {});
    // A newer recording against a build without a fingerprint supersedes the old
    // one.
    write(
        r,
        "visits/blog-2.json",
        &recording(
            "e2e/blog.spec.ts",
            "blog",
            Some("b2"),
            "passed",
            json!([get("document", "/")]),
        )
        .replace("2026-01-01", "2026-01-02"),
    );
    let text = explain(r, &same, "README.md", "e2e/blog.spec.ts");
    assert!(
        text.contains("recorded against build b2, whose fingerprint was not given"),
        "{text}"
    );
    // With that build's fingerprint the spec is skipped again.
    write(r, "b2.json", &fingerprint("b2", |_| {}));
    assert_eq!(
        select(r, &same, "README.md", &["--base-fingerprint", "b2.json"]),
        ALWAYS
    );
    // A recording that does not name its build.
    write(
        r,
        "visits/api.json",
        &recording(
            "e2e/api.spec.ts",
            "api",
            None,
            "passed",
            json!([get("api", "/api/hello")]),
        ),
    );
    let text = explain(r, &same, "README.md", "e2e/api.spec.ts");
    assert!(
        text.contains("does not say which build it ran against"),
        "{text}"
    );
    // A recording of another version of the spec file.
    write(
        r,
        "visits/about.json",
        &recording(
            "e2e/about.spec.ts",
            "about",
            Some("b1"),
            "passed",
            json!([get("document", "/about")]),
        )
        .replace(
            "\"specHash\":null",
            "\"specHash\":\"0000000000000000000000000000000000000000\"",
        ),
    );
    let text = explain(r, &same, "README.md", "e2e/about.spec.ts");
    assert!(
        text.contains("the spec file changed since it was recorded"),
        "{text}"
    );
}

#[test]
fn json_and_errors() {
    let d = repo();
    let r = d.path();
    write(r, "head.json", &fingerprint("b2", |_| {}));
    let out = ok(
        r,
        &[
            "affected",
            "--tests",
            "playwright",
            "--next-fingerprint",
            "head.json",
            "--visits",
            "visits",
            "--changed",
            "README.md",
            "--format",
            "json",
        ],
    );
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["specs"], 5);
    assert_eq!(v["selected"], 2);
    let blog = v["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["spec"] == "e2e/blog.spec.ts")
        .unwrap();
    assert_eq!(blog["run"], false);
    assert_eq!(blog["recorded"]["routes"], json!(["/", "/blog/[slug]"]));
    let fresh = v["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["spec"] == "e2e/fresh.spec.ts")
        .unwrap();
    assert_eq!(fresh["reasons"][0]["kind"], "no_recording");
    assert_eq!(fresh["reasons"][0]["text"], "no recording");

    let o = run(
        r,
        &[
            "affected",
            "--tests",
            "playwright",
            "--changed",
            "README.md",
        ],
    );
    // Without a fingerprint the selection is made from source
    // (tests/affected_playwright_source.rs): an unrelated change selects nothing.
    assert!(o.status.success());
    assert_eq!(String::from_utf8_lossy(&o.stdout), "");
    // Recordings need the fingerprint of the build under test.
    let o = run(
        r,
        &[
            "affected",
            "--tests",
            "playwright",
            "--visits",
            "visits",
            "--changed",
            "README.md",
        ],
    );
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("--next-fingerprint"));
    let o = run(
        r,
        &[
            "affected",
            "--tests",
            "playwright",
            "--tests",
            "jest",
            "--next-fingerprint",
            "head.json",
            "--changed",
            "README.md",
        ],
    );
    assert!(!o.status.success());
    let o = run(
        r,
        &[
            "affected",
            "--tests",
            "jest",
            "--visits",
            "visits",
            "--changed",
            "README.md",
        ],
    );
    assert!(!o.status.success());
    // --exit-code: 1 when a spec runs.
    let o = run(
        r,
        &[
            "affected",
            "--tests",
            "playwright",
            "--next-fingerprint",
            "head.json",
            "--visits",
            "visits",
            "--changed",
            "README.md",
            "--exit-code",
        ],
    );
    assert_eq!(o.status.code(), Some(1));
}

/// Without a recording, the spec's source can be enough: every navigation is a
/// literal URL that matches one route, and nothing global differs.
#[test]
fn a_spec_without_a_recording_is_read_from_its_source() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(
        r,
        "package.json",
        r#"{"name":"app","private":true,"dependencies":{"next":"16.0.0"}}"#,
    );
    write(r, ".gitignore", ".turbo/trace/\n");
    write(
        r,
        "next.config.mjs",
        "export default { reactStrictMode: true, async redirects() { return []; } };\n",
    );
    write(
        r,
        "app/page.tsx",
        "export default function Home() { return <main>home</main>; }\n",
    );
    write(
        r,
        "app/about/page.tsx",
        "export default function About() { return <main>about</main>; }\n",
    );
    write(
        r,
        "app/blog/[slug]/page.tsx",
        "export default function Post() { return <main>post</main>; }\n",
    );
    write(
        r,
        "playwright.config.ts",
        "export default { testDir: './e2e', use: { baseURL: 'http://localhost:3000' } };\n",
    );
    write(
        r,
        "e2e/literal.spec.ts",
        "import { test } from '@playwright/test';\ntest('about', async ({ page }) => {\n  await \
         page.goto('/about');\n});\n",
    );
    write(
        r,
        "e2e/computed.spec.ts",
        "import { test } from '@playwright/test';\nconst target = \
         process.env.TARGET;\ntest('somewhere', async ({ page }) => {\n  await \
         page.goto(target);\n});\n",
    );
    let with_app = |build: &str, edit: &dyn Fn(&mut Value)| {
        fingerprint(build, |v| {
            v["app"] = ".".into();
            edit(v);
        })
    };
    write(r, "base.json", &with_app("b1", &|_| {}));
    let run_with = |head: &str, extra: &[&str]| {
        write(r, "head.json", head);
        let mut args = vec![
            "affected",
            "--tests",
            "playwright",
            "--next-fingerprint",
            "head.json",
            "--base-fingerprint",
            "base.json",
            "--changed",
            "README.md",
        ];
        args.extend(extra);
        ok(r, &args)
    };
    // Nothing differs: the literal spec is skipped, the computed one runs.
    let same = with_app("b2", &|_| {});
    let text = run_with(&same, &["--explain", "e2e/literal.spec.ts"]);
    assert!(text.starts_with("skip e2e/literal.spec.ts\n"), "{text}");
    assert_eq!(run_with(&same, &[]).trim(), "e2e/computed.spec.ts");
    assert!(
        text.contains("no recording; read from its source it can reach 1 route(s): /about"),
        "{text}"
    );
    let text = run_with(&same, &["--explain", "e2e/computed.spec.ts"]);
    assert!(
        text.contains(
            "no recording; its source does not say where it navigates (e2e/computed.spec.ts:4"
        ),
        "{text}"
    );
    // The visited route changed.
    let head = with_app("b2", &|v| {
        v["routes"]["/about"]["server"]["hash"] = "s-new".into();
        v["routes"]["/about"]["own"] = "new".into();
        v["routes"]["/about"]["effective"] = "new".into();
    });
    let text = run_with(&head, &["--explain", "e2e/literal.spec.ts"]);
    assert!(
        text.contains("no recording; read from the source it visits /about, and /about changed"),
        "{text}"
    );
    // Another route changed: still skipped.
    let head = with_app("b2", &|v| {
        v["routes"]["/blog/[slug]"]["server"]["hash"] = "s-new".into();
        v["routes"]["/blog/[slug]"]["own"] = "new".into();
        v["routes"]["/blog/[slug]"]["effective"] = "new".into();
    });
    assert_eq!(run_with(&head, &[]).trim(), "e2e/computed.spec.ts");
    // Something global changed: the paths it requests are not known, so it runs.
    let head = with_app("b2", &|v| v["public"]["logo.png"] = "h2".into());
    let out = run_with(&head, &[]);
    assert!(out.contains("e2e/literal.spec.ts"), "{out}");
}

/// A helper that reads a file relative to the working directory does not make
/// the specs that import it uncertain: Playwright runs in its config's
/// directory, so the path is known, as it is for jest and vitest tests.
#[test]
fn working_directory_relative_reads_resolve_in_the_configs_directory() {
    let d = repo();
    let r = d.path();
    write(r, "fixtures/users.json", "[]\n");
    write(
        r,
        "e2e/helpers.ts",
        "import fs from 'node:fs';\nimport path from 'node:path';\nexport const login = async () \
         =>\n  JSON.parse(fs.readFileSync(path.join(process.cwd(), 'fixtures/users.json'), \
         'utf8'));\n",
    );
    let same = fingerprint("b2", |_| {});
    // Nothing the helper reads changed: the spec that imports it stays skipped.
    assert_eq!(select(r, &same, "README.md", &[]), ALWAYS);
    // The file it reads there changed.
    assert_eq!(
        select(r, &same, "fixtures/users.json", &[]),
        [
            "e2e/about.spec.ts",
            "e2e/broken.spec.ts",
            "e2e/fresh.spec.ts"
        ]
    );
}
