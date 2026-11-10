//! `turbotrace affected --since`: a changed dependency that a tool's config
//! names (`crates/turborepo-trace/src/confignames.rs`).
//!
//! The field that names the package decides who consumes it. A list of packages
//! that the tool treats specially when a module loads them
//! (`transpilePackages`, `transformIgnorePatterns`, `server.deps.inline`) adds
//! no consumer: the importers of the package are the consumers. A field through
//! which the tool loads the package itself (a compiler plugin, a test
//! environment), and every position that is not a decided field, is consumed by
//! whatever reads the config, as for an edit of the config.
//!
//! The Next.js config has the shape of a real application's: an object literal
//! in a variable, handed to wrappers from several packages before it is
//! exported, so the exported value cannot be evaluated. One pin per field of
//! the module's tables; the rules are argued from the tools' documentation, and
//! a runtime oracle (outside this repository) runs one field of each kind for
//! real.
//!
//! The repository has no `node_modules`, like a fresh clone: packages are
//! linked through the lockfile, and every edit changes the manifest and the
//! lockfile together.
// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{
    path::Path,
    process::{Command, Output},
};

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

fn run_env(root: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut c = Command::new(support::harness());
    c.args(args).current_dir(root);
    for (k, v) in env {
        c.env(k, v);
    }
    c.output().expect("run turbotrace")
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

/// The three packages and their dependencies at the base revision.
const SITE: &str = "apps/site";
const JESTAPP: &str = "apps/jestapp";
const VITEAPP: &str = "apps/viteapp";

const SITE_DEPS: &[&str] = &[
    "@acme/other",
    "@flow/web-shared",
    "date-lib",
    "icons-lib",
    "loader-pkg",
    "lodash-lib",
    "native-lib",
    "next",
    "plugin-pkg",
    "prod-lib",
    "swc-plugin-x",
    "unimported-lib",
];
const JESTAPP_DEPS: &[&str] = &["custom-env", "esm-lib", "mapped-lib"];
const VITEAPP_DEPS: &[&str] = &["deduped-lib", "inlined-lib", "vitest-env-pkg"];

fn manifest(name: &str, deps: &[(&str, String)]) -> String {
    let list: Vec<String> = deps
        .iter()
        .map(|(n, v)| format!("\"{n}\": \"{v}\""))
        .collect();
    format!(
        "{{\n  \"name\": \"{name}\",\n  \"version\": \"1.0.0\",\n  \"dependencies\": {{ {} \
         }}\n}}\n",
        list.join(", ")
    )
}

fn lock(importers: &[(&str, Vec<(&str, String)>)]) -> String {
    let key = |n: &str, v: &str| {
        if n.starts_with('@') {
            format!("'{n}@{v}'")
        } else {
            format!("{n}@{v}")
        }
    };
    let mut s = String::from(
        "lockfileVersion: '9.0'\n\nsettings:\n  autoInstallPeers: false\n  \
         excludeLinksFromLockfile: false\n\nimporters:\n\n  .: {}\n",
    );
    for (dir, deps) in importers {
        s.push_str(&format!("\n  {dir}:\n    dependencies:\n"));
        for (n, v) in deps {
            let k = if n.starts_with('@') {
                format!("'{n}'")
            } else {
                (*n).to_owned()
            };
            s.push_str(&format!(
                "      {k}:\n        specifier: {v}\n        version: {v}\n"
            ));
        }
    }
    s.push_str("\npackages:\n");
    for (_, deps) in importers {
        for (n, v) in deps {
            s.push_str(&format!(
                "\n  {}:\n    resolution: {{integrity: sha512-{}-{v}==}}\n",
                key(n, v),
                n.replace(['@', '/'], "")
            ));
        }
    }
    s.push_str("\nsnapshots:\n");
    for (_, deps) in importers {
        for (n, v) in deps {
            s.push_str(&format!("\n  {}: {{}}\n", key(n, v)));
        }
    }
    s
}

/// Writes the manifests and the lockfile with `bumped` at version 1.1.0 and
/// everything else at 1.0.0.
fn set(r: &Path, bumped: &str) {
    let deps = |names: &[&'static str]| -> Vec<(&'static str, String)> {
        names
            .iter()
            .map(|n| {
                let v = if *n == bumped { "1.1.0" } else { "1.0.0" };
                (*n, v.to_owned())
            })
            .collect()
    };
    let (site, jestapp, viteapp) = (deps(SITE_DEPS), deps(JESTAPP_DEPS), deps(VITEAPP_DEPS));
    write(r, "apps/site/package.json", &manifest("site", &site));
    write(
        r,
        "apps/jestapp/package.json",
        &manifest("jestapp", &jestapp),
    );
    write(
        r,
        "apps/viteapp/package.json",
        &manifest("viteapp", &viteapp),
    );
    write(
        r,
        "pnpm-lock.yaml",
        &lock(&[(SITE, site), (JESTAPP, jestapp), (VITEAPP, viteapp)]),
    );
}

const NEXT_JEST_CONFIG: &str = "import nextJest from 'next/jest';\nimport type { Config } from \
                                'jest';\n\nconst customConfig: Config = {\n  testEnvironment: \
                                'node',\n  testMatch: ['./**/*.test.{ts,tsx}'],\n};\n\nexport \
                                default async (): Promise<Config> => {\n  const config = (await \
                                nextJest({ dir: __dirname })(customConfig)()) as Config;\n  \
                                return config;\n};\n";

/// The shape of a real application's config (names changed): the object is a
/// variable, a later statement extends it, and wrappers of four packages (one
/// under a condition, loaded with `require`) stand between it and the export.
const NEXT_CONFIG: &str = r#"import path from 'node:path';
import { withPlugin } from 'plugin-pkg/next';
import { withOther } from '@acme/other';

const isDev = process.env.NODE_ENV === 'development';

const nextConfig = {
  reactStrictMode: true,
  transpilePackages:
    // These packages ship code that needs compiling.
    [
      '@acme/ui',
      '@flow/web-shared',
      'unimported-lib',
    ]
      .concat(
        isDev
          ? []
          : // These packages are bundled for production.
            ['prod-lib'],
      ),
  serverExternalPackages: ['native-lib'],
  experimental: {
    optimizePackageImports: ['icons-lib'],
    swcPlugins: [['swc-plugin-x', { mode: 'a' }]],
  },
  modularizeImports: {
    'lodash-lib': { transform: 'lodash-lib/{{member}}' },
  },
  images: { loaderFile: 'loader-pkg/next' },
  async rewrites() {
    return [{ source: '/a', destination: '/b' }];
  },
};
Object.assign(nextConfig.experimental, { optimizePackageImports: ['date-lib'] });

let config = withOther(withPlugin(nextConfig));

export default config;
"#;

const FLOW: &str = "apps/site/src/flow.test.ts";
const ICONS: &str = "apps/site/src/icons.test.ts";
const NATIVE: &str = "apps/site/src/native.test.ts";
const LODASH: &str = "apps/site/src/lodash.test.ts";
const DATE: &str = "apps/site/src/date.test.ts";
const PLAIN: &str = "apps/site/src/plain.test.ts";
const PROD: &str = "apps/site/src/prod.test.ts";
const SITE_TESTS: [&str; 7] = [DATE, FLOW, ICONS, LODASH, NATIVE, PLAIN, PROD];

const J_ESM: &str = "apps/jestapp/src/esm.test.js";
const J_PLAIN: &str = "apps/jestapp/src/plain.test.js";
const V_INLINED: &str = "apps/viteapp/src/inlined.test.ts";
const V_PLAIN: &str = "apps/viteapp/src/plain.test.ts";

/// A module that imports `package` and its test, and the path of the test.
fn importer(r: &Path, dir: &str, stem: &str, ext: &str, package: Option<&str>) {
    let (module, test) = match (package, ext) {
        (Some(p), "js") => (
            format!("module.exports = require('{p}');\n"),
            format!(
                "const v = require('./{stem}');\n\ntest('{stem}', () => {{\n  \
                 expect(v).toBeDefined();\n}});\n"
            ),
        ),
        (None, "js") => (
            "module.exports = 1;\n".to_owned(),
            format!(
                "const v = require('./{stem}');\n\ntest('{stem}', () => {{\n  \
                 expect(v).toBe(1);\n}});\n"
            ),
        ),
        (Some(p), _) => (
            format!("import {{ thing }} from '{p}';\n\nexport const v = thing;\n"),
            format!(
                "import {{ v }} from './{stem}';\n\ntest('{stem}', () => {{\n  \
                 expect(v).toBeDefined();\n}});\n"
            ),
        ),
        (None, _) => (
            "export const v = 1;\n".to_owned(),
            format!(
                "import {{ v }} from './{stem}';\n\ntest('{stem}', () => {{\n  \
                 expect(v).toBe(1);\n}});\n"
            ),
        ),
    };
    write(r, &format!("{dir}/src/{stem}.{ext}"), &module);
    write(r, &format!("{dir}/src/{stem}.test.{ext}"), &test);
}

fn repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"root","private":true}"#);
    write(r, "pnpm-workspace.yaml", "packages:\n  - 'apps/*'\n");
    write(r, ".gitignore", ".turbo/trace/\n");
    set(r, "");
    // A Next.js application tested through next/jest.
    write(r, "apps/site/jest.config.ts", NEXT_JEST_CONFIG);
    write(r, "apps/site/next.config.mjs", NEXT_CONFIG);
    importer(r, SITE, "flow", "ts", Some("@flow/web-shared"));
    importer(r, SITE, "icons", "ts", Some("icons-lib"));
    importer(r, SITE, "native", "ts", Some("native-lib"));
    importer(r, SITE, "lodash", "ts", Some("lodash-lib"));
    importer(r, SITE, "date", "ts", Some("date-lib"));
    importer(r, SITE, "plain", "ts", None);
    importer(r, SITE, "prod", "ts", Some("prod-lib"));
    // A plain jest project.
    write(
        r,
        "apps/jestapp/jest.config.js",
        "module.exports = {\n  testEnvironment: 'custom-env',\n  transformIgnorePatterns: \
         ['/node_modules/(?!(esm-lib|other-esm)/)'],\n  moduleNameMapper: { '^mapped$': \
         'mapped-lib/compat' },\n};\n",
    );
    importer(r, JESTAPP, "esm", "js", Some("esm-lib"));
    importer(r, JESTAPP, "plain", "js", None);
    // A vitest project.
    write(
        r,
        "apps/viteapp/vitest.config.ts",
        "import { defineConfig } from 'vitest/config';\n\nexport default defineConfig({\n  \
         resolve: { dedupe: ['deduped-lib'] },\n  test: {\n    environment: 'vitest-env-pkg',\n    \
         server: { deps: { inline: ['inlined-lib'] } },\n  },\n});\n",
    );
    importer(r, VITEAPP, "inlined", "ts", Some("inlined-lib"));
    importer(r, VITEAPP, "plain", "ts", None);
    git(r, &["init", "-q"]);
    git(r, &["add", "-A"]);
    git(r, &["commit", "-q", "-m", "base"]);
    d
}

const ARGS: [&str; 9] = [
    "affected",
    "--since",
    "HEAD",
    "--on-uncertain",
    "ignore",
    "--tests",
    "jest",
    "--tests",
    "vitest",
];

fn since_with(r: &Path, extra: &[&str], env: &[(&str, &str)]) -> Vec<String> {
    let mut args = ARGS.to_vec();
    args.extend(extra);
    let o = run_env(r, &args, env);
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

/// The tests selected when `package` is bumped.
fn bump(r: &Path, package: &str) -> Vec<String> {
    set(r, package);
    since_with(r, &[], &[])
}

fn stats(r: &Path) -> String {
    let mut args = ARGS.to_vec();
    args.push("--stats");
    String::from_utf8_lossy(&run_env(r, &args, &[]).stderr).into_owned()
}

/// The rule off: a package a config names has unknown consumers.
const OFF: [(&str, &str); 1] = [("TURBO_TRACE_DEBUG_CONFIG_NAMES", "0")];

#[test]
fn a_package_next_transpiles_is_consumed_by_its_importers() {
    let d = repo();
    let r = d.path();
    // The fixture's site: named in `transpilePackages`, imported by one module.
    assert_eq!(bump(r, "@flow/web-shared"), [FLOW]);
    let err = stats(r);
    assert!(
        err.contains(
            "apps/site/next.config.mjs names @flow/web-shared in transpilePackages[]: counted for \
             the files that load it"
        ),
        "{err}"
    );
    assert!(
        err.contains("every changed dependency has known consumers in the package"),
        "{err}"
    );
    // Without the rule the consumers are not known and the manifest counts whole.
    assert_eq!(since_with(r, &[], &OFF), SITE_TESTS);
    let o = run_env(r, &[&ARGS[..], &["--stats"]].concat(), &OFF);
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        err.contains(
            "whole-file change: changed dependenc(ies) whose consumers are not known: \
             @flow/web-shared"
        ),
        "{err}"
    );
    // The list is put together with `concat` under a condition, as in the real
    // config.
    assert_eq!(bump(r, "prod-lib"), [PROD]);
    assert!(stats(r).contains("names prod-lib in transpilePackages[]"));
    // Named there and imported by nothing: the lockfile diff says what the install
    // changes, nothing in the package can observe the version.
    assert!(bump(r, "unimported-lib").is_empty());
    assert_eq!(since_with(r, &[], &OFF), SITE_TESTS);
}

#[test]
fn the_other_next_fields_that_only_importers_consume() {
    let d = repo();
    let r = d.path();
    assert_eq!(bump(r, "native-lib"), [NATIVE]);
    assert!(stats(r).contains("in serverExternalPackages[]"));
    assert_eq!(bump(r, "icons-lib"), [ICONS]);
    assert!(stats(r).contains("in experimental.optimizePackageImports[]"));
    assert_eq!(bump(r, "lodash-lib"), [LODASH]);
    assert!(
        stats(r).contains("in modularizeImports.<package>, modularizeImports.lodash-lib.transform")
    );
}

#[test]
fn a_package_the_tool_loads_is_consumed_by_the_readers_of_the_config() {
    let d = repo();
    let r = d.path();
    // A compiler plugin runs on every file: the jest tests of the next/jest project
    // read the config, the other projects do not.
    assert_eq!(bump(r, "swc-plugin-x"), SITE_TESTS);
    let err = stats(r);
    assert!(
        err.contains(
            "apps/site/next.config.mjs names swc-plugin-x (experimental.swcPlugins[][]): counted \
             for everything that reads the config"
        ),
        "{err}"
    );
    // A field this rule does not decide.
    assert_eq!(bump(r, "loader-pkg"), SITE_TESTS);
    // The decided key in a position that is not the config's own nesting (the
    // argument of `Object.assign`).
    assert_eq!(bump(r, "date-lib"), SITE_TESTS);
    assert!(stats(r).contains("names date-lib (optimizePackageImports[])"));
}

#[test]
fn a_package_the_config_imports() {
    let d = repo();
    let r = d.path();
    // A wrapper runs when the config loads, and next/jest loads the config: it
    // returns the object whose fields the transform of every test gets. The
    // config counts as changed for the tests that read it, as for an edit of
    // the config.
    assert_eq!(bump(r, "plugin-pkg"), SITE_TESTS);
    let err = stats(r);
    assert!(
        err.contains(
            "apps/site/next.config.mjs imports plugin-pkg, which runs when the config loads: \
             counted for the 7 node(s) that read the config's values without loading it"
        ),
        "{err}"
    );
    // `--explain` carries the sentence.
    let args = [&ARGS[..], &["--explain", PLAIN]].concat();
    let explained = String::from_utf8(run_env(r, &args, &[]).stdout).unwrap();
    assert!(
        explained.contains(
            "apps/site/next.config.mjs imports plugin-pkg, which runs when the config loads"
        ),
        "{explained}"
    );
    // The A/B switch assumes that the package leaves those values alone: then only
    // what loads the config is seeded (nothing here), and the notes say what
    // was assumed.
    let assume = [("TURBO_TRACE_DEBUG_CONFIG_IMPORTS", "0")];
    assert!(since_with(r, &[], &assume).is_empty());
    let o = run_env(r, &[&ARGS[..], &["--stats"]].concat(), &assume);
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        err.contains(
            "assumed: plugin-pkg, which apps/site/next.config.mjs imports, changes what loads the \
             config, not the values that 7 node(s) read from it"
        ),
        "{err}"
    );
    // `--strict` never assumes it.
    assert_eq!(since_with(r, &["--strict"], &assume), SITE_TESTS);
    // A package that only a jest config imports reaches the tests through their
    // config edge either way.
    assert_eq!(bump(r, "next"), SITE_TESTS);
    assert_eq!(since_with(r, &[], &assume), SITE_TESTS);
}

#[test]
fn jest_and_vitest_fields() {
    let d = repo();
    let r = d.path();
    // jest: a pattern that lets a package through the transform.
    assert_eq!(bump(r, "esm-lib"), [J_ESM]);
    assert!(
        stats(r).contains("apps/jestapp/jest.config.js names esm-lib in transformIgnorePatterns[]")
    );
    assert_eq!(since_with(r, &[], &OFF), [J_ESM, J_PLAIN]);
    // jest: an environment and a mapper target are loaded for every test.
    assert_eq!(bump(r, "custom-env"), [J_ESM, J_PLAIN]);
    assert_eq!(bump(r, "mapped-lib"), [J_ESM, J_PLAIN]);
    // vitest: an inlined dependency.
    assert_eq!(bump(r, "inlined-lib"), [V_INLINED]);
    assert!(
        stats(r).contains(
            "apps/viteapp/vitest.config.ts names inlined-lib in test.server.deps.inline[]"
        )
    );
    assert_eq!(since_with(r, &[], &OFF), [V_INLINED, V_PLAIN]);
    // vitest: an environment, and `resolve.dedupe`, which redirects imports
    // everywhere.
    assert_eq!(bump(r, "vitest-env-pkg"), [V_INLINED, V_PLAIN]);
    assert_eq!(bump(r, "deduped-lib"), [V_INLINED, V_PLAIN]);
}
