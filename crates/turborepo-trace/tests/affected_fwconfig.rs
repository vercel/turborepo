//! `turbotrace affected` end to end over test-runner configs whose values need
//! evaluation: one small repository per config shape that used to raise an
//! uncertainty on every test of its project (which selects them all on every
//! change). For each: an unrelated edit selects no test, and an edit of
//! something the config value names selects the tests it reaches.
// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{path::Path, process::Command};

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

/// A minimal installed package.
fn package(root: &Path, name: &str, files: &[(&str, &str)]) {
    write(
        root,
        &format!("node_modules/{name}/package.json"),
        &format!(r#"{{"name":"{name}","version":"1.0.0","main":"index.js"}}"#),
    );
    write(
        root,
        &format!("node_modules/{name}/index.js"),
        "module.exports = {};\n",
    );
    for (rel, content) in files {
        write(root, &format!("node_modules/{name}/{rel}"), content);
    }
}

fn affected(root: &Path, framework: &str, changed: &[&str]) -> Vec<String> {
    let mut args = vec!["affected", "--tests", framework];
    for c in changed {
        args.push("--changed");
        args.push(c);
    }
    // These tests are about how open uncertainty travels; their fixtures use a read
    // with a computed path as the uncertain site. Such reads are bounded by
    // names by default (tests/affected_names.rs), so that layer is off here.
    let o = Command::new(support::harness())
        .args(&args)
        .env("TURBO_TRACE_DEBUG_NAMES", "0")
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

/// A vite alias list whose first `find` is `new RegExp(...)` over a template
/// string with a negative lookahead built from a list (`.map(x =>
/// x.replace(/\./g, '\\.')).join('|')`), and whose replacement is
/// `${projectRoot}$1`.
#[test]
fn vitest_alias_built_with_new_regexp() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"docs","version":"1.0.0"}"#);
    package(r, "vitest", &[("config.js", "module.exports = {};\n")]);
    write(
        r,
        "vitest.config.ts",
        r#"import { fileURLToPath } from 'node:url';
import { defineConfig } from 'vitest/config';

const projectRoot = fileURLToPath(new URL('.', import.meta.url));

const GENERATED = ['app/routes.mjs', 'lib/valid-paths.mjs'];

const generatedPattern = GENERATED.map((path) =>
  path.replace(/\./g, '\\.'),
).join('|');

const alias = [
  {
    find: new RegExp(`^#/(?!(?:${generatedPattern})$)(.*)$`),
    replacement: `${projectRoot}$1`,
  },
  {
    find: 'server-only',
    replacement: fileURLToPath(
      new URL('./test/server-only-stub.ts', import.meta.url),
    ),
  },
  {
    find: /^next\/font\/(?:local|google)$/,
    replacement: fileURLToPath(new URL('./test/font-stub.ts', import.meta.url)),
  },
];

export default defineConfig({
  resolve: { alias },
  test: { setupFiles: ['./vitest-setup.ts'] },
});
"#,
    );
    write(r, "vitest-setup.ts", "export {};\n");
    write(r, "test/server-only-stub.ts", "export {};\n");
    write(r, "test/font-stub.ts", "export default () => ({});\n");
    write(r, "lib/util.ts", "export const util = 1;\n");
    write(r, "lib/other.ts", "export const other = 1;\n");
    write(
        r,
        "lib/fonts.ts",
        "import local from 'next/font/local';\nexport const f = local();\n",
    );
    write(
        r,
        "lib/util.test.ts",
        "import 'server-only';\nimport { util } from '#/lib/util';\ntest('util', () => \
         expect(util).toBe(1));\n",
    );
    write(
        r,
        "lib/fonts.test.ts",
        "import { f } from '#/lib/fonts';\ntest('fonts', () => expect(f).toBeDefined());\n",
    );
    write(
        r,
        "lib/routes.test.ts",
        "import { vi } from 'vitest';\nvi.mock('#/app/routes.mjs', () => ({ routes: [] \
         }));\nimport { routes } from '#/app/routes.mjs';\ntest('routes', () => \
         expect(routes).toEqual([]));\n",
    );
    let all = [
        "lib/fonts.test.ts",
        "lib/routes.test.ts",
        "lib/util.test.ts",
    ];
    assert_eq!(affected(r, "vitest", &["vitest.config.ts"]), all);
    assert_eq!(affected(r, "vitest", &["vitest-setup.ts"]), all);
    // Files reached through the regex alias select the tests whose imports it
    // rewrites (nothing else resolves `#/lib/util` here: there is no tsconfig).
    assert_eq!(
        affected(r, "vitest", &["lib/util.ts"]),
        ["lib/util.test.ts"]
    );
    assert_eq!(
        affected(r, "vitest", &["lib/fonts.ts"]),
        ["lib/fonts.test.ts"]
    );
    // The stubs the other aliases name select the tests that import the aliased
    // module, not every test of the config.
    assert_eq!(
        affected(r, "vitest", &["test/server-only-stub.ts"]),
        ["lib/util.test.ts"]
    );
    assert_eq!(
        affected(r, "vitest", &["test/font-stub.ts"]),
        ["lib/fonts.test.ts"]
    );
    // Unrelated edits select nothing: no test of the project is uncertain.
    assert!(affected(r, "vitest", &["lib/other.ts"]).is_empty());
    assert!(affected(r, "vitest", &["README.md"]).is_empty());
}

const A_TEST: &str = "import { a } from './a';\ntest('a', () => expect(a).toBe(1));\n";
const B_TEST: &str = "test('b', () => expect(1).toBe(1));\n";

/// An alias target is a resolution rule, not a file the config reads. With
/// `alias: { '@': path.resolve(__dirname, 'src') }` every test used to depend
/// on every file below `src` through its config.
#[test]
fn vitest_alias_target_directory_is_not_a_dependency_of_every_test() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"app","version":"1.0.0"}"#);
    package(r, "vitest", &[("config.js", "module.exports = {};\n")]);
    write(
        r,
        "vitest.config.ts",
        "import path from 'node:path';\nimport { defineConfig } from 'vitest/config';\n\nexport \
         default defineConfig({\n  resolve: { alias: { '@': path.resolve(__dirname, 'src') } \
         },\n});\n",
    );
    write(r, "src/a.ts", "export const a = 1;\n");
    write(r, "src/c.ts", "export const c = 1;\n");
    write(r, "src/a.test.ts", A_TEST);
    write(r, "src/b.test.ts", B_TEST);
    write(
        r,
        "src/c.test.ts",
        "import { c } from '@/c';\ntest('c', () => expect(c).toBe(1));\n",
    );
    assert_eq!(affected(r, "vitest", &["src/a.ts"]), ["src/a.test.ts"]);
    // Through the alias.
    assert_eq!(affected(r, "vitest", &["src/c.ts"]), ["src/c.test.ts"]);
    assert_eq!(affected(r, "vitest", &["src/b.test.ts"]), ["src/b.test.ts"]);
    assert!(affected(r, "vitest", &["README.md"]).is_empty());
    assert_eq!(
        affected(r, "vitest", &["vitest.config.ts"]),
        ["src/a.test.ts", "src/b.test.ts", "src/c.test.ts"]
    );
}

/// The same for the shapes of a real app config: `new URL('./lib',
/// import.meta.url)`, alias files, `require.resolve(x, { paths: [dir] })` and
/// an alias `customResolver` that is `require.resolve` of its argument. An
/// uncertain file below an alias directory does not make the tests that never
/// load it uncertain.
#[test]
fn vitest_config_path_values_of_an_app() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"app","version":"1.0.0"}"#);
    package(r, "vitest", &[("config.js", "module.exports = {};\n")]);
    write(
        r,
        "vitest.config.ts",
        r#"import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { defineConfig } from 'vitest/config';

const require = createRequire(import.meta.url);

export default defineConfig({
  resolve: {
    alias: {
      '@lib': fileURLToPath(new URL('./lib', import.meta.url)),
    },
  },
  test: {
    setupFiles: ['./test/setup.ts'],
    alias: [
      {
        find: '@acme/client',
        replacement: require.resolve('@acme/client', {
          paths: [path.resolve(__dirname, './packages/content')],
        }),
      },
      {
        find: /^@acme\/content\/(.+)$/,
        replacement: '@acme/content/$1',
        customResolver: (specifier: string): string => {
          return require.resolve(specifier);
        },
      },
      { find: '@', replacement: path.resolve(__dirname, './src') },
      {
        find: 'next/font/google',
        replacement: path.resolve(__dirname, './test/mocks/font.ts'),
      },
    ],
  },
});
"#,
    );
    write(r, "test/setup.ts", "export {};\n");
    write(
        r,
        "test/mocks/font.ts",
        "export const Inter = () => ({});\n",
    );
    write(
        r,
        "packages/content/package.json",
        r#"{"name":"@acme/content","version":"1.0.0"}"#,
    );
    write(r, "packages/content/posts.ts", "export const posts = [];\n");
    write(r, "lib/util.ts", "export const util = 1;\n");
    write(r, "src/a.ts", "export const a = 1;\n");
    write(
        r,
        "src/fonts.ts",
        "import { Inter } from 'next/font/google';\nexport const inter = Inter();\n",
    );
    // An uncertain file below the alias directory: a computed read.
    write(
        r,
        "src/report.ts",
        "import { readFileSync } from 'node:fs';\nexport const read = (name: string) => \
         readFileSync(name, 'utf8');\n",
    );
    write(r, "src/a.test.ts", A_TEST);
    write(r, "src/b.test.ts", B_TEST);
    write(
        r,
        "src/fonts.test.ts",
        "import { inter } from '@/fonts';\nimport { util } from '@lib/util';\ntest('f', () => \
         expect(inter && util).toBeTruthy());\n",
    );
    write(
        r,
        "src/report.test.ts",
        "import { read } from './report';\ntest('r', () => expect(read).toBeDefined());\n",
    );
    // A null change selects only the test that loads the uncertain file.
    assert_eq!(
        affected(r, "vitest", &["README.md"]),
        ["src/report.test.ts"]
    );
    assert_eq!(
        affected(r, "vitest", &["src/a.ts"]),
        ["src/a.test.ts", "src/report.test.ts"]
    );
    // Alias targets select the tests that import through the alias.
    assert_eq!(
        affected(r, "vitest", &["test/mocks/font.ts"]),
        ["src/fonts.test.ts", "src/report.test.ts"]
    );
    assert_eq!(
        affected(r, "vitest", &["lib/util.ts"]),
        ["src/fonts.test.ts", "src/report.test.ts"]
    );
    // The directory `require.resolve` searches from is not read either.
    assert_eq!(
        affected(r, "vitest", &["packages/content/posts.ts"]),
        ["src/report.test.ts"]
    );
    // Setup files stay inputs of every test.
    assert_eq!(
        affected(r, "vitest", &["test/setup.ts"]),
        [
            "src/a.test.ts",
            "src/b.test.ts",
            "src/fonts.test.ts",
            "src/report.test.ts"
        ]
    );
}

/// What keeps the dependency: a path that also reaches code the evaluator
/// cannot follow (an untracked helper, a plugin's options), a path in a field
/// the adapter does not model, and a real read of the directory.
#[test]
fn vitest_config_paths_with_other_uses_stay_dependencies() {
    for (name, config) in [
        (
            "helper",
            "const SRC = path.resolve(__dirname, 'src');\nconst extra = collect(SRC);\nexport \
             default defineConfig({ resolve: { alias: { '@': SRC } }, define: { extra } });\n",
        ),
        (
            "plugin",
            "export default defineConfig({\n  plugins: [copy({ from: path.resolve(__dirname, \
             'src') })],\n  resolve: { alias: { '@': path.resolve(__dirname, 'src') } },\n});\n",
        ),
        (
            "other field",
            "const SRC = path.resolve(__dirname, 'src');\nexport default defineConfig({ \
             publicDir: SRC, resolve: { alias: { '@': SRC } } });\n",
        ),
        (
            "read",
            "import fs from 'node:fs';\nconst SRC = path.resolve(__dirname, 'src');\nconst names \
             = fs.readdirSync(SRC);\nexport default defineConfig({ resolve: { alias: { '@': SRC } \
             }, define: { names } });\n",
        ),
    ] {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        write(r, "package.json", r#"{"name":"app","version":"1.0.0"}"#);
        package(r, "vitest", &[("config.js", "module.exports = {};\n")]);
        package(r, "helpers", &[]);
        write(
            r,
            "vitest.config.ts",
            &format!(
                "import path from 'node:path';\nimport {{ defineConfig }} from \
                 'vitest/config';\nimport {{ collect, copy }} from 'helpers';\n{config}"
            ),
        );
        write(r, "src/a.ts", "export const a = 1;\n");
        write(r, "src/a.test.ts", A_TEST);
        write(r, "src/b.test.ts", B_TEST);
        assert_eq!(
            affected(r, "vitest", &["src/a.ts"]),
            ["src/a.test.ts", "src/b.test.ts"],
            "{name}"
        );
    }
}

/// The same for jest: `rootDir`, `roots` and `moduleNameMapper` targets. A
/// `modulePaths` directory stays a dependency of every test: the graph does not
/// resolve imports through it, so the directory as a whole is what covers them.
#[test]
fn jest_config_path_values_are_not_dependencies_of_every_test() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"svc","version":"1.0.0"}"#);
    write(
        r,
        "jest.config.js",
        "const path = require('path');\n\nmodule.exports = {\n  rootDir: \
         path.resolve(__dirname),\n  roots: [path.join(__dirname, 'src')],\n  modulePaths: \
         [path.resolve(__dirname, 'shared')],\n  moduleNameMapper: {\n    '^@/(.*)$': \
         path.resolve(__dirname, 'src') + '/$1',\n    '^lib/(.*)$': `${path.resolve(__dirname, \
         'lib')}/$1`,\n    '^config$': path.resolve(__dirname, 'mocks/config.js'),\n  },\n};\n",
    );
    write(r, "lib/x.js", "module.exports = 1;\n");
    write(r, "lib/y.js", "module.exports = 2;\n");
    write(r, "mocks/config.js", "module.exports = {};\n");
    write(r, "shared/helper.js", "module.exports = 1;\n");
    write(r, "src/a.js", "module.exports = 1;\n");
    write(r, "src/c.js", "module.exports = require('config');\n");
    write(
        r,
        "src/a.test.js",
        "require('./a');\ntest('a', () => {});\n",
    );
    write(r, "src/b.test.js", "test('b', () => {});\n");
    write(
        r,
        "src/c.test.js",
        "require('@/c');\nrequire('helper');\nrequire('lib/x');\ntest('c', () => {});\n",
    );
    assert_eq!(affected(r, "jest", &["src/a.js"]), ["src/a.test.js"]);
    assert_eq!(affected(r, "jest", &["src/c.js"]), ["src/c.test.js"]);
    assert_eq!(affected(r, "jest", &["mocks/config.js"]), ["src/c.test.js"]);
    // A mapper target directory written as `${path.resolve(__dirname, 'lib')}/$1`:
    // the file an import maps to selects its importer, another file of the
    // directory selects nothing.
    assert_eq!(affected(r, "jest", &["lib/x.js"]), ["src/c.test.js"]);
    assert!(affected(r, "jest", &["lib/y.js"]).is_empty());
    let all = ["src/a.test.js", "src/b.test.js", "src/c.test.js"];
    assert_eq!(affected(r, "jest", &["shared/helper.js"]), all);
    assert_eq!(affected(r, "jest", &["jest.config.js"]), all);
}

/// Vite plugins: `react()` redirects no import, and a loader plugin for `.wgsl`
/// modules makes nothing uncertain by itself: a shader's import lines are read
/// (`affected_shaders.rs`). A shader that includes another one with a
/// preprocessor line, which is not read, makes what loads it uncertain, with or
/// without the plugin. A plugin the adapter does not know keeps every test of
/// the project uncertain.
#[test]
fn vitest_plugins_known_loader_and_unknown() {
    let repo = |plugins: &str, shader: &str| {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        write(r, "package.json", r#"{"name":"app","version":"1.0.0"}"#);
        package(r, "vitest", &[("config.js", "module.exports = {};\n")]);
        write(
            r,
            "vitest.config.ts",
            &format!(
                "import {{ defineConfig }} from 'vitest/config';\nimport react from \
                 '@vitejs/plugin-react';\nimport wgslVitePlugin from \
                 '@vgpu/wgsl/loader-vite';\nimport custom from 'vite-plugin-custom';\n\nexport \
                 default defineConfig({{\n  plugins: [{plugins}],\n  test: {{ environment: \
                 'jsdom' }},\n}});\n"
            ),
        );
        write(r, "src/a.ts", "export const a = 1;\n");
        write(r, "src/a.test.ts", A_TEST);
        write(r, "src/b.test.ts", B_TEST);
        write(r, "src/main.wgsl", shader);
        write(r, "src/common.wgsl", "const PI = 3.14;\n");
        write(
            r,
            "src/render.ts",
            "import shader from './main.wgsl';\nexport const source: string = shader;\n",
        );
        write(
            r,
            "src/render.test.ts",
            "import { source } from './render';\ntest('r', () => expect(source).toBeDefined());\n",
        );
        d
    };
    const PLAIN: &str = "struct U { t: f32 }\n";
    const INCLUDES: &str = "#import \"./common.wgsl\"\nstruct U { t: f32 }\n";
    // Only `react()`: nothing is uncertain.
    let d = repo("react()", PLAIN);
    let r = d.path();
    assert!(affected(r, "vitest", &["README.md"]).is_empty());
    assert_eq!(affected(r, "vitest", &["src/a.ts"]), ["src/a.test.ts"]);
    // With the shader loader, the same.
    let d = repo("react(), wgslVitePlugin({ minify: true })", PLAIN);
    let r = d.path();
    assert!(affected(r, "vitest", &["README.md"]).is_empty());
    assert_eq!(
        affected(r, "vitest", &["src/main.wgsl"]),
        ["src/render.test.ts"]
    );
    // A shader with an `#import` line: the test that loads it is uncertain (what
    // that line includes is not traced), the others are not.
    let d = repo("react(), wgslVitePlugin({ minify: true })", INCLUDES);
    let r = d.path();
    assert_eq!(
        affected(r, "vitest", &["README.md"]),
        ["src/render.test.ts"]
    );
    assert_eq!(
        affected(r, "vitest", &["src/a.ts"]),
        ["src/a.test.ts", "src/render.test.ts"]
    );
    assert_eq!(
        affected(r, "vitest", &["src/common.wgsl"]),
        ["src/render.test.ts"]
    );
    // Without the loader plugin too: the line is in the file whoever compiles it.
    let d = repo("react()", INCLUDES);
    let r = d.path();
    assert_eq!(
        affected(r, "vitest", &["README.md"]),
        ["src/render.test.ts"]
    );
    // An unknown plugin: every test of the project.
    let d = repo("react(), custom()", PLAIN);
    let r = d.path();
    assert_eq!(
        affected(r, "vitest", &["README.md"]),
        ["src/a.test.ts", "src/b.test.ts", "src/render.test.ts"]
    );
}

/// A jest `moduleNameMapper` target computed with `fs.realpathSync`: the real
/// path of a dependency's link in `node_modules`, then a sibling package in the
/// store. The mapper used to be unknown, which made every test of the package
/// uncertain.
#[cfg(unix)]
#[test]
fn jest_mapper_target_through_realpath() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"root","version":"1.0.0"}"#);
    write(
        r,
        "packages/chat/package.json",
        r#"{"name":"chat","version":"1.0.0"}"#,
    );
    write(
        r,
        "packages/chat/jest.config.js",
        r#"const path = require('path');

module.exports = {
  moduleNameMapper: {
    '^@scope/shared$': path.resolve(
      require('fs').realpathSync(
        path.resolve(__dirname, 'node_modules/@scope/x'),
      ),
      '../shared/dist/index.js',
    ),
  },
};
"#,
    );
    let store = "node_modules/.pnpm/@scope+x@1.0.0/node_modules/@scope";
    for (pkg, file) in [("x", "index.js"), ("shared", "dist/index.js")] {
        write(
            r,
            &format!("{store}/{pkg}/package.json"),
            &format!(r#"{{"name":"@scope/{pkg}","version":"1.0.0"}}"#),
        );
        write(r, &format!("{store}/{pkg}/{file}"), "exports.chat = 1;\n");
    }
    std::fs::create_dir_all(r.join("packages/chat/node_modules/@scope")).unwrap();
    turborepo_trace_core::os::symlink(
        r.join(store).join("x"),
        r.join("packages/chat/node_modules/@scope/x"),
    )
    .unwrap();
    write(r, "packages/chat/src/a.js", "module.exports = 1;\n");
    write(
        r,
        "packages/chat/src/a.test.js",
        "require('./a');\ntest('a', () => {});\n",
    );
    write(
        r,
        "packages/chat/src/chat.test.js",
        "require('@scope/shared');\ntest('chat', () => {});\n",
    );
    // No test is uncertain.
    assert!(affected(r, "jest", &["README.md"]).is_empty());
    assert_eq!(
        affected(r, "jest", &["packages/chat/src/a.js"]),
        ["packages/chat/src/a.test.js"]
    );
    // (The mapped module is a file of an installed package: its changes arrive as
    // lockfile changes, not as path changes.)
    assert_eq!(
        affected(r, "jest", &["packages/chat/jest.config.js"]),
        [
            "packages/chat/src/a.test.js",
            "packages/chat/src/chat.test.js"
        ]
    );
}

/// A reporter runs in the runner and observes results: an uncertain read below
/// it does not make the tests uncertain, while an edit of the reporter still
/// selects them. The same uncertain helper imported for a config value keeps
/// the tests uncertain.
#[test]
fn reporter_uncertainty_does_not_make_tests_uncertain() {
    let repo = |config: &str| {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        write(r, "package.json", r#"{"name":"app","version":"1.0.0"}"#);
        package(r, "vitest", &[("config.js", "module.exports = {};\n")]);
        write(
            r,
            "vitest.config.ts",
            &format!("import {{ defineConfig }} from 'vitest/config';\n{config}"),
        );
        write(
            r,
            "tools/metrics-reporter.ts",
            "import { load } from './store';\nexport default class MetricsReporter {\n  \
             onFinished() {\n    load('out', 'metrics.json');\n  }\n}\n",
        );
        write(
            r,
            "tools/settings.ts",
            "import { load } from './store';\nexport const timeout = Number(load('.', \
             'timeout'));\n",
        );
        // An open computed read.
        write(
            r,
            "tools/store.ts",
            "import { readFileSync } from 'node:fs';\nimport { join } from 'node:path';\nexport \
             const load = (dir: string, name: string) => readFileSync(join(dir, name), 'utf8');\n",
        );
        write(r, "src/a.ts", "export const a = 1;\n");
        write(r, "src/a.test.ts", A_TEST);
        d
    };
    let test = ["src/a.test.ts"];
    for (name, config) in [
        (
            "imported reporter class",
            "import MetricsReporter from './tools/metrics-reporter';\n\nexport default \
             defineConfig({\n  test: { reporters: ['default', new MetricsReporter()] },\n});\n",
        ),
        (
            "reporter path",
            "export default defineConfig({\n  test: { reporters: ['default', \
             './tools/metrics-reporter.ts'] },\n});\n",
        ),
    ] {
        let d = repo(config);
        let r = d.path();
        assert!(affected(r, "vitest", &["README.md"]).is_empty(), "{name}");
        assert_eq!(affected(r, "vitest", &["src/a.ts"]), test, "{name}");
        // The reporter is still an input of the run.
        assert_eq!(
            affected(r, "vitest", &["tools/metrics-reporter.ts"]),
            test,
            "{name}"
        );
        assert_eq!(affected(r, "vitest", &["tools/store.ts"]), test, "{name}");
    }
    // Not a reporter: the helper computes a config value.
    let d = repo(
        "import { timeout } from './tools/settings';\n\nexport default defineConfig({ test: { \
         testTimeout: timeout } });\n",
    );
    assert_eq!(affected(d.path(), "vitest", &["README.md"]), test);
    // A binding that is a reporter and something else.
    let d = repo(
        "import MetricsReporter from './tools/metrics-reporter';\n\nexport default \
         defineConfig({\n  test: { reporters: [new MetricsReporter()], sequence: { sequencer: \
         MetricsReporter } },\n});\n",
    );
    assert_eq!(affected(d.path(), "vitest", &["README.md"]), test);
}

/// A jest config whose `testMatch` is read exactly and matches no file has no
/// tests. The Playwright specs next to it are not jest tests (discovery used to
/// fall back to jest's default patterns, which match `*.spec.ts`, with an
/// uncertainty on each). A file the pattern matches becomes a test when it
/// appears.
#[test]
fn jest_exact_test_match_that_finds_nothing() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"site","version":"1.0.0"}"#);
    write(r, "jest.config.js", "module.exports = {};\n");
    write(r, "src/a.ts", "export const a = 1;\n");
    write(r, "src/a.test.ts", A_TEST);
    write(
        r,
        "playwright/jest.config.ts",
        "import type { Config } from 'jest';\n\nconst jestConfig: Config = {\n  testMatch: \
         ['<rootDir>/**/*.test.ts'],\n  transform: {\n    '^.+\\\\.ts$': '@swc/jest',\n  \
         },\n};\n\nmodule.exports = jestConfig;\n",
    );
    write(
        r,
        "playwright/shared/selectors.ts",
        "export const selectors = { login: '#login' };\n",
    );
    write(
        r,
        "playwright/e2e/login.spec.ts",
        "import { selectors } from '../shared/selectors';\ntest('login', () => \
         expect(selectors).toBeDefined());\n",
    );
    assert_eq!(affected(r, "jest", &["src/a.ts"]), ["src/a.test.ts"]);
    assert!(affected(r, "jest", &["README.md"]).is_empty());
    assert!(affected(r, "jest", &["playwright/shared/selectors.ts"]).is_empty());
    assert!(affected(r, "jest", &["playwright/e2e/login.spec.ts"]).is_empty());
    // A unit test of the helpers appears: it is a jest test of that config.
    write(
        r,
        "playwright/shared/selectors.test.ts",
        "import { selectors } from './selectors';\ntest('s', () => \
         expect(selectors).toBeDefined());\n",
    );
    assert_eq!(
        affected(r, "jest", &["playwright/shared/selectors.ts"]),
        ["playwright/shared/selectors.test.ts"]
    );
}

/// `testMatch` with plain groups (`*.(test|spec).[jt]s?(x)`) and a negated
/// pattern: the tests are discovered as jest discovers them, and none is
/// uncertain.
#[test]
fn jest_test_match_with_plain_groups() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"storage","version":"1.0.0"}"#);
    write(
        r,
        "jest.config.js",
        "module.exports = {\n  testMatch: [\n    '**/__tests__/**/*.(test|spec).[jt]s?(x)',\n    \
         '**/*.(test|spec).[jt]s?(x)',\n    '!**/__tests__/integration/**',\n  ],\n};\n",
    );
    write(r, "src/a.ts", "export const a = 1;\n");
    write(r, "src/a.test.ts", A_TEST);
    write(r, "src/b.spec.ts", B_TEST);
    write(r, "src/__tests__/integration/c.test.ts", A_TEST);
    assert_eq!(affected(r, "jest", &["src/a.ts"]), ["src/a.test.ts"]);
    assert!(affected(r, "jest", &["README.md"]).is_empty());
    assert_eq!(
        affected(r, "jest", &["jest.config.js"]),
        ["src/a.test.ts", "src/b.spec.ts"]
    );
}

/// A jest `resolver` module that is the default resolver with another export
/// condition for one request: no test is uncertain because of it, unless it
/// imports that request and the request resolves into the repository. Another
/// kind of resolver keeps every test uncertain.
#[test]
fn jest_resolver_that_only_changes_conditions() {
    let repo = |resolver: &str| {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        write(r, "package.json", r#"{"name":"site","version":"1.0.0"}"#);
        write(
            r,
            "jest.config.js",
            "module.exports = { resolver: '<rootDir>/jest.resolver.cjs' };\n",
        );
        write(r, "jest.resolver.cjs", resolver);
        package(r, "tree-walker", &[]);
        // A workspace package, linked into node_modules.
        write(
            r,
            "packages/shared/package.json",
            r#"{"name":"@site/shared","version":"1.0.0","main":"index.js"}"#,
        );
        write(r, "packages/shared/index.js", "module.exports = 1;\n");
        std::fs::create_dir_all(r.join("node_modules/@site")).unwrap();
        #[cfg(unix)]
        turborepo_trace_core::os::symlink(
            r.join("packages/shared"),
            r.join("node_modules/@site/shared"),
        )
        .unwrap();
        write(r, "src/a.js", "module.exports = require('tree-walker');\n");
        write(
            r,
            "src/a.test.js",
            "require('./a');\ntest('a', () => {});\n",
        );
        write(r, "src/b.test.js", "test('b', () => {});\n");
        write(
            r,
            "src/c.test.js",
            "require('@site/shared');\ntest('c', () => {});\n",
        );
        d
    };
    let conditions = |requests: &str| {
        format!(
            "module.exports = (request, options) =>\n  options.defaultResolver(request, {{\n    \
             ...options,\n    conditions: {requests} ? [...(options.conditions ?? []), 'import'] \
             : options.conditions,\n  }});\n"
        )
    };
    let all = ["src/a.test.js", "src/b.test.js", "src/c.test.js"];
    // Only an installed package: nothing is uncertain.
    let d = repo(&conditions("request === 'tree-walker'"));
    let r = d.path();
    assert!(affected(r, "jest", &["README.md"]).is_empty());
    assert_eq!(affected(r, "jest", &["src/a.js"]), ["src/a.test.js"]);
    // The resolver is an input of every test.
    assert_eq!(affected(r, "jest", &["jest.resolver.cjs"]), all);
    // A request that resolves into the repository: its importers are uncertain.
    #[cfg(unix)]
    {
        let d = repo(&conditions(
            "request === 'tree-walker' || request === '@site/shared'",
        ));
        assert_eq!(
            affected(d.path(), "jest", &["README.md"]),
            ["src/c.test.js"]
        );
    }
    // A resolver that does its own lookup: every test.
    let d = repo("module.exports = (request, options) => require('./lookup')(request, options);\n");
    assert_eq!(affected(d.path(), "jest", &["README.md"]), all);
}
