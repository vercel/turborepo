//! What `next/jest` reads besides the jest config
//! (`crates/turborepo-trace-frameworks/src/jest.rs`,
//! `crates/turborepo-trace/src/nextconfigdiff.rs`).
//!
//! `nextJest({ dir })(custom)` loads the Next.js config of `dir` (its
//! `transpilePackages`, `modularizeImports`, `compiler` and
//! `experimental.swcPlugins` become options of the SWC transform) and the env
//! files of test mode. Neither is imported by a test. What the config loads
//! while it runs is an input of the tests like the config itself; a module that
//! only route functions use, and whose load only defines things, is not.
// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{path::Path, process::Command};

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

const JEST_CONFIG: &str = "import nextJest from 'next/jest';\nimport type { Config } from \
                           'jest';\n\nconst customConfig: Config = {\n  testEnvironment: \
                           'jsdom',\n  testMatch: ['./**/*.test.{ts,tsx}'],\n};\n\nexport default \
                           async (): Promise<Config> => {\n  const config = (await nextJest({ \
                           dir: __dirname })(customConfig)()) as Config;\n  return config;\n};\n";

fn repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"root","private":true}"#);
    write(r, "pnpm-workspace.yaml", "packages:\n  - 'apps/*'\n");
    write(
        r,
        "apps/site/package.json",
        r#"{"name":"site","version":"1.0.0","scripts":{"test":"jest"}}"#,
    );
    write(r, "apps/site/jest.config.ts", JEST_CONFIG);
    write(
        r,
        "apps/site/next.config.mjs",
        "import { redirects } from './config/redirects.mjs';\n\nexport default {\n  \
         transpilePackages: ['@scope/ui'],\n  modularizeImports: { '@scope/icons': { transform: \
         '@scope/icons/dist/{{member}}' } },\n  async redirects() {\n    return redirects;\n  \
         },\n};\n",
    );
    write(
        r,
        "apps/site/config/redirects.mjs",
        "export const redirects = [{ source: '/a', destination: '/b', permanent: false }];\n",
    );
    write(r, "apps/site/.env.test", "API_URL=http://localhost:3000\n");
    write(r, "apps/site/lib/a.ts", "export const a = 1;\n");
    write(
        r,
        "apps/site/lib/a.test.ts",
        "import { a } from './a';\n\ntest('a', () => {\n  expect(a).toBe(1);\n});\n",
    );
    // A plain jest project next to it.
    write(
        r,
        "apps/plain/package.json",
        r#"{"name":"plain","version":"1.0.0","scripts":{"test":"jest"}}"#,
    );
    write(r, "apps/plain/jest.config.js", "module.exports = {};\n");
    write(r, "apps/plain/next.config.mjs", "export default {};\n");
    write(r, "apps/plain/src/p.js", "module.exports = 1;\n");
    write(
        r,
        "apps/plain/src/p.test.js",
        "const p = require('./p');\n\ntest('p', () => {\n  expect(p).toBe(1);\n});\n",
    );
    d
}

fn selected(root: &Path, path: &str) -> Vec<String> {
    let args = [
        "affected",
        "--tests",
        "jest",
        "--changed",
        path,
        "--on-uncertain",
        "ignore",
    ];
    let o = Command::new(support::harness())
        .args(args)
        .current_dir(root)
        .output()
        .expect("run the harness");
    assert!(
        o.status.success(),
        "harness {args:?} failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

const SITE_TEST: &[&str] = &["apps/site/lib/a.test.ts"];

#[test]
fn the_next_config_and_the_test_env_files_are_inputs_of_the_tests() {
    let d = repo();
    let r = d.path();
    assert_eq!(selected(r, "apps/site/next.config.mjs"), SITE_TEST);
    assert_eq!(selected(r, "apps/site/.env.test"), SITE_TEST);
    // An env file that appears.
    write(r, "apps/site/.env", "FLAG=1\n");
    assert_eq!(selected(r, "apps/site/.env"), SITE_TEST);
}

#[test]
fn without_a_base_revision_a_module_the_next_config_loads_counts() {
    let d = repo();
    let r = d.path();
    // `--changed` names a path and nothing else: whether the module did more than
    // define route rules before the edit cannot be shown, so the config's
    // process loaded a module that changed.
    assert_eq!(selected(r, "apps/site/config/redirects.mjs"), SITE_TEST);
}

fn git(root: &Path, args: &[&str]) {
    let o = Command::new("git")
        .args(["-c", "user.email=t@t", "-c", "user.name=t"])
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
}

/// The jest tests affected by the working tree against `HEAD`.
fn since(root: &Path, env: &[(&str, &str)]) -> Vec<String> {
    let args = [
        "affected",
        "--tests",
        "jest",
        "--since",
        "HEAD",
        "--on-uncertain",
        "ignore",
    ];
    let mut c = Command::new(support::harness());
    c.args(args).current_dir(root);
    for (k, v) in env {
        c.env(k, v);
    }
    let o = c.output().expect("run the harness");
    assert!(
        o.status.success(),
        "harness {args:?} failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// An app's config: values `next/jest` reads, code that runs at load, route
/// rules inside `rewrites()`, and a plugin around it.
fn app_config(rewrites: &str, transpile: &str) -> String {
    format!(
        "import fs from 'node:fs';\nimport {{ withPlugin }} from 'next-plugin';\n\nif \
         (!process.env.API_URL) {{\n  console.warn('API_URL is missing');\n}}\n\nconst nextConfig \
         = {{\n  transpilePackages: [{transpile}],\n  async rewrites() {{\n    return {{\n      \
         beforeFiles: [\n{rewrites}      ],\n    }};\n  }},\n  async headers() {{\n    return [{{ \
         source: '/(.*)', headers: [{{ key: 'x-frame-options', value: 'DENY' }}] }}];\n  \
         }},\n}};\n\nexport default withPlugin(nextConfig);\n"
    )
}

const RULE_A: &str = "        { source: '/a/:path*', destination: 'https://a.example/:path*' },\n";
const RULE_B: &str = "        {\n          source: '/:team/:project/c/:id',\n          destination: 'https://b.example/:team/:project/:id',\n        },\n";
const CONFIG_TEST: &str = "apps/site/test/next-config.test.ts";
const A_TEST: &str = "apps/site/lib/a.test.ts";

/// [`repo`] with [`app_config`] and a test of the config itself, committed.
fn app() -> tempfile::TempDir {
    let d = repo();
    let r = d.path();
    write(r, ".gitignore", ".turbo/trace/\n");
    write(
        r,
        "apps/site/next.config.mjs",
        &app_config(&format!("{RULE_A}{RULE_B}"), "'@scope/ui'"),
    );
    write(
        r,
        CONFIG_TEST,
        "import nextConfig from '../next.config.mjs';\n\ntest('rewrites', async () => {\n  \
         expect(await nextConfig.rewrites()).toBeDefined();\n});\n",
    );
    git(r, &["init", "-q"]);
    git(r, &["add", "-A"]);
    git(r, &["commit", "-q", "-m", "base"]);
    d
}

#[test]
fn a_route_rule_of_the_next_config_is_not_an_input_of_the_tests() {
    let d = app();
    let r = d.path();
    // One rule of `rewrites()` goes away. `next/jest` loads the config and never
    // calls `rewrites()`: the jest config it returns is the same. The test that
    // imports the config calls it.
    write(
        r,
        "apps/site/next.config.mjs",
        &app_config(RULE_A, "'@scope/ui'"),
    );
    assert_eq!(since(r, &[]), [CONFIG_TEST]);
    assert_eq!(
        since(r, &[("TURBO_TRACE_DEBUG_CONFIG_FIELDS", "0")]),
        [A_TEST, CONFIG_TEST]
    );
    // A value `next/jest` hands to the transform.
    write(
        r,
        "apps/site/next.config.mjs",
        &app_config(&format!("{RULE_A}{RULE_B}"), "'@scope/ui', '@scope/icons'"),
    );
    assert_eq!(since(r, &[]), [A_TEST, CONFIG_TEST]);
    // Code that runs when the config loads.
    write(
        r,
        "apps/site/next.config.mjs",
        &app_config(&format!("{RULE_A}{RULE_B}"), "'@scope/ui'")
            .replace("console.warn(", "throw new Error("),
    );
    assert_eq!(since(r, &[]), [A_TEST, CONFIG_TEST]);
    // A rule that does not parse breaks the load.
    write(
        r,
        "apps/site/next.config.mjs",
        &app_config(
            "        { source: '/a' destination: '/b' },\n",
            "'@scope/ui'",
        ),
    );
    assert_eq!(since(r, &[]), [A_TEST, CONFIG_TEST]);
}

#[test]
fn a_jest_project_without_next_jest_does_not_read_them() {
    let d = repo();
    let r = d.path();
    assert!(selected(r, "apps/plain/next.config.mjs").is_empty());
    // Not loaded in test mode.
    write(r, "apps/site/.env.local", "SECRET=1\n");
    assert!(selected(r, "apps/site/.env.local").is_empty());
}

/// A config that loads modules in every way the table of `nextconfigdiff` tells
/// apart: `packages.mjs` computes a field the transform reads (through
/// `shared.mjs`), `routes.mjs` is used only inside `redirects()` and only
/// defines a function, `flagged.mjs` is used only inside `rewrites()` but sets
/// a variable when it loads, `lazy.mjs` is loaded inside `rewrites()`,
/// `data.json` is read when the config loads.
const LOADING_CONFIG: &str = r#"import fs from 'node:fs';
import { withPlugin } from 'next-plugin';
import { packages } from './config/packages.mjs';
import { routes } from './config/routes.mjs';
import { flagged } from './config/flagged.mjs';

const data = JSON.parse(fs.readFileSync(new URL('./config/data.json', import.meta.url), 'utf8'));

const csp = "script-src 'self' 'unsafe-eval'";

const nextConfig = {
  transpilePackages: packages,
  env: { NAME: data.name, CSP: csp },
  async redirects() {
    return routes();
  },
  async rewrites() {
    const { lazy } = await import('./config/lazy.mjs');
    return [...flagged, ...lazy];
  },
};

export default withPlugin(nextConfig);
"#;

const ROUTES: &str = "apps/site/config/routes.mjs";

fn loading_app() -> tempfile::TempDir {
    let d = repo();
    let r = d.path();
    write(r, ".gitignore", ".turbo/trace/\n");
    write(r, "apps/site/next.config.mjs", LOADING_CONFIG);
    write(
        r,
        "apps/site/config/packages.mjs",
        "import { shared } from './shared.mjs';\n\nexport const packages = ['@scope/ui', \
         ...shared];\n",
    );
    write(
        r,
        "apps/site/config/shared.mjs",
        "export const shared = ['@scope/icons'];\n",
    );
    write(
        r,
        ROUTES,
        "import { paths } from './paths.mjs';\n\nexport function routes() {\n  return [{ source: \
         paths.old, destination: '/b', permanent: false }];\n}\n",
    );
    write(
        r,
        "apps/site/config/paths.mjs",
        "export const paths = { old: '/a' };\n",
    );
    write(
        r,
        "apps/site/config/flagged.mjs",
        "process.env.FLAGGED = '1';\n\nexport const flagged = [{ source: '/f', destination: '/g' \
         }];\n",
    );
    write(
        r,
        "apps/site/config/lazy.mjs",
        "export const lazy = [{ source: '/l', destination: '/m' }];\n",
    );
    write(r, "apps/site/config/data.json", "{ \"name\": \"site\" }\n");
    git(r, &["init", "-q"]);
    git(r, &["add", "-A"]);
    git(r, &["commit", "-q", "-m", "base"]);
    d
}

fn stats(root: &Path, extra: &[&str]) -> String {
    let mut args = vec![
        "affected",
        "--tests",
        "jest",
        "--since",
        "HEAD",
        "--on-uncertain",
        "ignore",
        "--stats",
    ];
    args.extend(extra);
    let o = Command::new(support::harness())
        .args(&args)
        .current_dir(root)
        .output()
        .expect("run the harness");
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn reset(r: &Path) {
    git(r, &["checkout", "-q", "--", "."]);
}

#[test]
fn what_the_next_config_loads_is_an_input_of_the_tests() {
    let d = loading_app();
    let r = d.path();
    let off = [("TURBO_TRACE_DEBUG_CONFIG_IMPORTS", "0")];
    // A module whose value becomes a field the transform reads.
    write(
        r,
        "apps/site/config/packages.mjs",
        "import { shared } from './shared.mjs';\n\nexport const packages = ['@scope/ui', \
         '@scope/more', ...shared];\n",
    );
    assert_eq!(since(r, &[]), SITE_TEST);
    let err = stats(r, &[]);
    assert!(
        err.contains(
            "next config apps/site/next.config.mjs: apps/site/config/packages.mjs that it loads \
             is affected: an input change for 1 jest test(s) of its next/jest project"
        ),
        "{err}"
    );
    // The reading before: what the config imports was not followed.
    assert!(since(r, &off).is_empty());
    reset(r);
    // A module that module loads in turn.
    write(
        r,
        "apps/site/config/shared.mjs",
        "export const shared = ['@scope/other'];\n",
    );
    assert_eq!(since(r, &[]), SITE_TEST);
    assert!(since(r, &off).is_empty());
    reset(r);
    // A file the config reads when it loads.
    write(r, "apps/site/config/data.json", "{ \"name\": \"other\" }\n");
    assert_eq!(since(r, &[]), SITE_TEST);
    reset(r);
    // A change that is only in types and comments is no change of a loaded module.
    write(
        r,
        "apps/site/config/shared.mjs",
        "// the shared packages\nexport const shared = ['@scope/icons'];\n",
    );
    assert!(since(r, &[]).is_empty());
}

#[test]
fn a_module_only_route_functions_use_is_not_an_input() {
    let d = loading_app();
    let r = d.path();
    let all = [("TURBO_TRACE_DEBUG_CONFIG_IMPORTS", "all")];
    // One more redirect: `next/jest` never calls `redirects()`, and loading the
    // module (and the one it loads) only defines values.
    write(
        r,
        ROUTES,
        "import { paths } from './paths.mjs';\n\nexport function routes() {\n  return [\n    { \
         source: paths.old, destination: '/b', permanent: false },\n    { source: '/c', \
         destination: '/d', permanent: true },\n  ];\n}\n",
    );
    assert!(since(r, &[]).is_empty());
    let err = stats(r, &[]);
    assert!(
        err.contains(
            "next config apps/site/next.config.mjs: apps/site/config/routes.mjs changed, which \
             the config loads through apps/site/config/routes.mjs: not an input change for 1 jest \
             test(s): only route functions use it, which next/jest never calls, and loading it \
             does the same as before"
        ),
        "{err}"
    );
    // Without the narrowing, and under --strict (a wrapper might call the
    // function).
    assert_eq!(since(r, &all), SITE_TEST);
    assert!(stats(r, &["--strict"]).contains(A_TEST));
    reset(r);
    // The module it loads in turn, the same way.
    write(
        r,
        "apps/site/config/paths.mjs",
        "export const paths = { old: '/a2' };\n",
    );
    assert!(since(r, &[]).is_empty());
    assert_eq!(since(r, &all), SITE_TEST);
    reset(r);
    // A load inside a route function never runs.
    write(
        r,
        "apps/site/config/lazy.mjs",
        "export const lazy = [{ source: '/l2', destination: '/m' }];\n",
    );
    assert!(since(r, &[]).is_empty());
    assert_eq!(since(r, &all), SITE_TEST);
    reset(r);
    // The edit makes loading the module do something: the tests' process sees it.
    write(
        r,
        ROUTES,
        "import { paths } from './paths.mjs';\n\nprocess.env.ROUTES = 'on';\n\nexport function \
         routes() {\n  return [{ source: paths.old, destination: '/b', permanent: false }];\n}\n",
    );
    assert_eq!(since(r, &[]), SITE_TEST);
    // And the other way round: the edit removes what loading it did.
    git(r, &["add", "-A"]);
    git(r, &["commit", "-q", "-m", "routes sets a variable"]);
    write(
        r,
        ROUTES,
        "import { paths } from './paths.mjs';\n\nexport function routes() {\n  return [{ source: \
         paths.old, destination: '/b', permanent: false }];\n}\n",
    );
    assert_eq!(since(r, &[]), SITE_TEST);
    reset(r);
    // A module only a route function uses, whose load sets a variable: it counts.
    write(
        r,
        "apps/site/config/flagged.mjs",
        "process.env.FLAGGED = '2';\n\nexport const flagged = [{ source: '/f', destination: '/g' \
         }];\n",
    );
    assert_eq!(since(r, &[]), SITE_TEST);
    reset(r);
    // A module that starts to load a package: what loading a package does is not
    // known.
    write(
        r,
        "apps/site/config/paths.mjs",
        "import 'some-package';\n\nexport const paths = { old: '/a' };\n",
    );
    assert_eq!(since(r, &[]), SITE_TEST);
    reset(r);
    // The config starts to use the module outside the route function.
    write(
        r,
        "apps/site/next.config.mjs",
        &LOADING_CONFIG.replace("CSP: csp },", "CSP: csp, N: String(routes.length) },"),
    );
    assert_eq!(since(r, &[]), SITE_TEST);
    git(r, &["add", "-A"]);
    git(r, &["commit", "-q", "-m", "the top level uses routes"]);
    write(
        r,
        ROUTES,
        "import { paths } from './paths.mjs';\n\nexport function routes() {\n  return [{ source: \
         paths.old, destination: '/b2', permanent: false }];\n}\n",
    );
    assert_eq!(since(r, &[]), SITE_TEST);
}

/// The helper of a real application's config (names changed): it takes its own
/// location when it loads, and a function that only `rewrites()` calls reads a
/// file and lists the application's directories with a class of a package.
const ZONES: &str = r#"import fs from 'node:fs';
import { ZoneConfig } from '@acme/zones/config';

const appRoot = new URL('./', import.meta.url);

export function childPaths() {
  const config = ZoneConfig.fromFile(fs.readFileSync(new URL('zones.jsonc', appRoot), 'utf8'));
  return segments(new URL('./app', appRoot)).filter((s) => config.has(s));
}

function segments(dir) {
  const out = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    out.push(entry.name);
  }
  return out;
}
"#;

#[test]
fn a_helper_that_reads_the_application_inside_a_function() {
    let d = repo();
    let r = d.path();
    write(r, ".gitignore", ".turbo/trace/\n");
    write(
        r,
        "apps/site/next.config.mjs",
        "import { withPlugin } from 'next-plugin';\nimport { childPaths } from \
         './zones.mjs';\n\n// As in the real config: its own location, a name of every path of \
         the application.\nconst here = new URL('.', import.meta.url);\n\nconst nextConfig = {\n  \
         transpilePackages: ['@scope/ui'],\n  env: { HERE: here.pathname },\n  async rewrites() \
         {\n    return childPaths().map((source) => ({ source, destination: '/zone' }));\n  \
         },\n};\n\nexport default withPlugin(nextConfig);\n",
    );
    write(r, "apps/site/zones.mjs", ZONES);
    write(r, "apps/site/zones.jsonc", "{ \"zones\": [] }\n");
    write(
        r,
        "apps/site/app/docs/page.js",
        "export default function Page() {\n  return null;\n}\n",
    );
    git(r, &["init", "-q"]);
    git(r, &["add", "-A"]);
    git(r, &["commit", "-q", "-m", "base"]);
    let all = [("TURBO_TRACE_DEBUG_CONFIG_IMPORTS", "all")];
    // What the function reads and lists is not read when the config loads: the
    // helper's top level calls nothing of the repository, so `childPaths` runs
    // only in `rewrites()`.
    write(
        r,
        "apps/site/app/blog/page.js",
        "export default function Page() {\n  return null;\n}\n",
    );
    write(r, "apps/site/zones.jsonc", "{ \"zones\": [\"blog\"] }\n");
    assert!(since(r, &[]).is_empty(), "{}", stats(r, &[]));
    // Nor does the config's location name what the helper's function lists (a read
    // with a computed path, bounded by names): no such read runs when the
    // config loads.
    assert!(since_default(r).is_empty(), "{}", stats(r, &[]));
    // Counting every load, the helper is affected through what its function reads.
    assert_eq!(since(r, &all), SITE_TEST);
    reset(r);
    std::fs::remove_dir_all(r.join("apps/site/app/blog")).unwrap();
    // An edit inside the function.
    write(
        r,
        "apps/site/zones.mjs",
        &ZONES.replace(
            "out.push(entry.name);",
            "if (entry.isDirectory()) out.push(entry.name);",
        ),
    );
    assert!(since(r, &[]).is_empty(), "{}", stats(r, &[]));
    assert!(stats(r, &[]).contains(
        "apps/site/zones.mjs changed, which the config loads through apps/site/zones.mjs: not an \
         input change for 1 jest test(s)"
    ));
    assert_eq!(since(r, &all), SITE_TEST);
    // An edit of what runs when the helper loads.
    write(
        r,
        "apps/site/zones.mjs",
        &ZONES.replace(
            "new URL('./', import.meta.url)",
            "new URL('../', import.meta.url)",
        ),
    );
    assert_eq!(since(r, &[]), SITE_TEST);
    // The helper starts to call its function when it loads: what the function reads
    // is read at load, and nothing about the helper is left out.
    write(
        r,
        "apps/site/zones.mjs",
        &format!("{ZONES}\nexport const paths = childPaths();\n"),
    );
    git(r, &["add", "-A"]);
    git(
        r,
        &[
            "commit",
            "-q",
            "-m",
            "the helper calls its function at load",
        ],
    );
    write(r, "apps/site/zones.jsonc", "{ \"zones\": [\"docs\"] }\n");
    assert_eq!(since(r, &[]), SITE_TEST);
}

#[test]
fn a_file_the_config_reads_in_the_working_directory() {
    let d = repo();
    let r = d.path();
    write(r, ".gitignore", ".turbo/trace/\n");
    // As in a real application's config: a file of the working directory, read when
    // the config loads. The config loads in the jest process, whose working
    // directory is the tests'.
    write(
        r,
        "apps/site/next.config.mjs",
        "import fs from 'node:fs';\nimport path from 'node:path';\n\nconst slugs = \
         JSON.parse(fs.readFileSync(path.join(process.cwd(), 'slugs.json'), 'utf8'));\n\nexport \
         default {\n  transpilePackages: ['@scope/ui'],\n  env: { SLUGS: String(slugs.length) \
         },\n};\n",
    );
    write(r, "apps/site/slugs.json", "[\"a\"]\n");
    write(r, "apps/site/lib/b.ts", "export const b = 2;\n");
    git(r, &["init", "-q"]);
    git(r, &["add", "-A"]);
    git(r, &["commit", "-q", "-m", "base"]);
    // Another file of the package is not that file.
    write(r, "apps/site/lib/b.ts", "export const b = 3;\n");
    assert!(since(r, &[]).is_empty(), "{}", stats(r, &[]));
    reset(r);
    write(r, "apps/site/slugs.json", "[\"a\", \"b\"]\n");
    assert_eq!(since(r, &[]), SITE_TEST, "{}", stats(r, &[]));
    assert!(since(r, &[("TURBO_TRACE_DEBUG_CONFIG_IMPORTS", "0")]).is_empty());
}

/// `affected --since HEAD` under the default policy for open uncertainty.
fn since_default(root: &Path) -> Vec<String> {
    let args = ["affected", "--tests", "jest", "--since", "HEAD"];
    let o = Command::new(support::harness())
        .args(args)
        .current_dir(root)
        .output()
        .expect("run the harness");
    assert!(
        o.status.success(),
        "harness {args:?} failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn a_directory_the_config_lists_in_the_working_directory() {
    let d = repo();
    let r = d.path();
    write(r, ".gitignore", ".turbo/trace/\n");
    write(
        r,
        "apps/site/next.config.mjs",
        "import fs from 'node:fs';\nimport path from 'node:path';\n\nconst pages = \
         fs.readdirSync(path.join(process.cwd(), 'content'));\n\nexport default {\n  \
         transpilePackages: ['@scope/ui'],\n  env: { PAGES: String(pages.length) },\n};\n",
    );
    write(r, "apps/site/content/a.md", "a\n");
    write(r, "apps/site/contents.md", "not in the directory\n");
    git(r, &["init", "-q"]);
    git(r, &["add", "-A"]);
    git(r, &["commit", "-q", "-m", "base"]);
    // A path that only starts like the directory is not in it.
    write(r, "apps/site/contents.md", "still not\n");
    assert!(since(r, &[]).is_empty(), "{}", stats(r, &[]));
    assert!(since_default(r).is_empty(), "{}", stats(r, &[]));
    reset(r);
    // A file that appears in the listed directory.
    write(r, "apps/site/content/b.md", "b\n");
    assert_eq!(since(r, &[]), SITE_TEST, "{}", stats(r, &[]));
    assert_eq!(since_default(r), SITE_TEST);
}

#[test]
fn a_computed_path_in_the_working_directory_is_bounded_by_names() {
    let d = repo();
    let r = d.path();
    write(r, ".gitignore", ".turbo/trace/\n");
    // The name is not known: which file of the working directory the config reads
    // is open, and so is whether a change in the package reaches the tests.
    write(
        r,
        "apps/site/next.config.mjs",
        "import fs from 'node:fs';\nimport os from 'node:os';\nimport path from \
         'node:path';\n\nconst data = fs.readFileSync(path.join(process.cwd(), 'data', \
         os.hostname() + '.json'), 'utf8');\n\nexport default {\n  transpilePackages: \
         ['@scope/ui'],\n  env: { DATA: data },\n};\n",
    );
    write(r, "apps/site/data/a.json", "{}\n");
    write(r, "apps/site/lib/b.ts", "export const b = 2;\n");
    git(r, &["init", "-q"]);
    git(r, &["add", "-A"]);
    git(r, &["commit", "-q", "-m", "base"]);
    // The read is bounded by the names of the code loaded with it ('data'): the
    // names layer runs for the config as it does for an entry.
    write(r, "apps/site/data/a.json", "{\"a\": 1}\n");
    assert_eq!(since_default(r), SITE_TEST, "{}", stats(r, &[]));
    assert_eq!(since(r, &[]), SITE_TEST, "{}", stats(r, &[]));
    assert!(since(r, &[("TURBO_TRACE_DEBUG_CONFIG_IMPORTS", "0")]).is_empty());
    reset(r);
    // The working directory is a name too, of every path in it: as for a test with
    // such a read, any file of the application counts.
    write(r, "apps/site/lib/b.ts", "export const b = 3;\n");
    assert_eq!(since_default(r), SITE_TEST, "{}", stats(r, &[]));
    reset(r);
    // A file of another package is not named.
    write(r, "apps/plain/src/p.js", "module.exports = 2;\n");
    assert_eq!(since_default(r), ["apps/plain/src/p.test.js"]);
}
