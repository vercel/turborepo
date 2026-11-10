//! A file path handed to an imported function
//! (`crates/turborepo-trace-lang-js/src/extract/patharg.rs`).
//!
//! The shape comes from a real app: a jest test loads the app's routing config
//! through a helper of a third-party package,
//! `loadConfigForEdge('./routes.jsonc')`, and checks every route against a list
//! in code. A change that adds a route to the config fails the test;
//! the read happens inside the package, so the path literal is the only link.
// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{path::Path, process::Command};

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

fn repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"root","private":true}"#);
    write(r, "pnpm-workspace.yaml", "packages:\n  - 'apps/*'\n");
    write(
        r,
        "apps/site/package.json",
        r#"{"name":"site","version":"1.0.0","scripts":{"test":"jest"},"dependencies":{"@scope/routing":"2.4.0"}}"#,
    );
    write(r, "apps/site/jest.config.js", "module.exports = {};\n");
    write(
        r,
        "apps/site/routes.jsonc",
        "{\n  // child applications\n  \"applications\": { \"docs\": { \"routing\": [{ \"paths\": \
         [\"/docs\", \"/docs/:path*\"] }] } }\n}\n",
    );
    write(
        r,
        "apps/site/app/lib/forbidden-slugs.mjs",
        "export const FORBIDDEN_SLUGS = new Set(['docs', 'shop']);\n",
    );
    write(
        r,
        "apps/site/app/lib/forbidden-slugs.test.ts",
        "import { loadConfigForEdge } from '@scope/routing/next/testing';\nimport { \
         FORBIDDEN_SLUGS } from './forbidden-slugs.mjs';\n\ndescribe('forbidden slugs', () => {\n  \
         it('has the first path segment of every route', () => {\n    const config = \
         loadConfigForEdge('./routes.jsonc');\n    const paths = \
         config.getChildApplications().flatMap((app) => app.routing.flatMap((group) => \
         group.paths));\n    const errors = paths.map((p) => p.split('/')[1]).filter((s) => \
         !FORBIDDEN_SLUGS.has(s));\n    expect(errors).toEqual([]);\n  });\n});\n",
    );
    write(r, "apps/site/app/lib/other.ts", "export const other = 1;\n");
    write(
        r,
        "apps/site/app/lib/other.test.ts",
        "import { other } from './other';\n\ntest('other', () => {\n  \
         expect(other).toBe(1);\n});\n",
    );
    // A file of the same name in another app.
    write(
        r,
        "apps/docs/package.json",
        r#"{"name":"docs","version":"1.0.0","scripts":{"test":"jest"}}"#,
    );
    write(r, "apps/docs/jest.config.js", "module.exports = {};\n");
    write(r, "apps/docs/routes.jsonc", "{}\n");
    write(r, "apps/docs/src/d.js", "module.exports = 1;\n");
    write(
        r,
        "apps/docs/src/d.test.js",
        "const d = require('./d');\n\ntest('d', () => {\n  expect(d).toBe(1);\n});\n",
    );
    d
}

fn selected(root: &Path, path: &str, extra: &[&str]) -> Vec<String> {
    let mut args = vec!["affected", "--tests", "jest", "--changed", path];
    args.extend_from_slice(extra);
    let o = Command::new(support::harness())
        .args(&args)
        .current_dir(root)
        .output()
        .expect("run turbotrace");
    assert!(
        o.status.success(),
        "turbotrace {args:?} failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

const SLUGS_TEST: &str = "apps/site/app/lib/forbidden-slugs.test.ts";

#[test]
fn the_config_handed_to_a_package_helper_selects_the_test() {
    let d = repo();
    let r = d.path();
    for policy in [&[][..], &["--on-uncertain", "ignore"][..]] {
        // The file the test names, relative to the directory jest runs in.
        assert_eq!(
            selected(r, "apps/site/routes.jsonc", policy),
            [SLUGS_TEST],
            "{policy:?}"
        );
        // A file of the same name elsewhere, and unrelated files of both apps.
        assert!(
            selected(r, "apps/docs/routes.jsonc", policy).is_empty(),
            "{policy:?}"
        );
        assert_eq!(
            selected(r, "apps/site/app/lib/other.ts", policy),
            ["apps/site/app/lib/other.test.ts"],
            "{policy:?}"
        );
        assert_eq!(
            selected(r, "apps/docs/src/d.js", policy),
            ["apps/docs/src/d.test.js"],
            "{policy:?}"
        );
    }
}

/// A loader that resolves against its caller: the same literal names a file
/// next to the test. Creating it selects the test.
#[test]
fn a_file_added_next_to_the_caller_selects_the_test() {
    let d = repo();
    let r = d.path();
    assert!(selected(r, "apps/site/app/lib/unrelated.json", &[]).is_empty());
    write(r, "apps/site/app/lib/routes.jsonc", "{}\n");
    assert_eq!(
        selected(
            r,
            "apps/site/app/lib/routes.jsonc",
            &["--on-uncertain", "ignore"]
        ),
        [SLUGS_TEST]
    );
}
