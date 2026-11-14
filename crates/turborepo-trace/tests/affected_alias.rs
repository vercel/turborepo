//! `affected --since`: an alias entry of a vitest config selects the
//! tests that can load a file with a request the alias changes, not every test
//! of the config (`crates/turborepo-trace/src/aliasdiff.rs`).
// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{path::Path, process::Command};

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
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

/// The vitest tests affected by the working tree against `HEAD`.
fn since(root: &Path, env: &[(&str, &str)]) -> Vec<String> {
    let args = [
        "affected",
        "--tests",
        "vitest",
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

/// A config with an alias table.
fn with_aliases(entries: &str, environment: &str) -> String {
    format!(
        "import {{ fileURLToPath }} from 'node:url';\nimport {{ defineConfig }} from \
         'vitest/config';\n\nexport default defineConfig({{\n  resolve: {{\n    alias: {{\n      \
         '@': fileURLToPath(new URL('.', import.meta.url)),\n{entries}    }},\n  }},\n  test: \
         {{\n    // Server code runs under node.\n    environment: '{environment}',\n  }},\n}});\n"
    )
}

/// A config without one.
const NO_ALIASES: &str = "import { defineConfig } from 'vitest/config';\n\nexport default \
                          defineConfig({\n  test: {\n    globals: true,\n    environment: \
                          'node',\n  },\n});\n";

/// The same config with a `resolve.alias` block for the stub (and the import
/// its value needs).
const STUB_BLOCK: &str = "import { fileURLToPath } from 'node:url';\nimport { defineConfig } from \
                          'vitest/config';\n\nexport default defineConfig({\n  resolve: {\n    \
                          alias: {\n      'server-only': fileURLToPath(\n        new \
                          URL('./test/server-only-stub.ts', import.meta.url),\n      ),\n    },\n  \
                          },\n  test: {\n    globals: true,\n    environment: 'node',\n  },\n});\n";

const STUB_ENTRY: &str = "      'server-only': fileURLToPath(\n        new \
                          URL('./test/server-only-stub.ts', import.meta.url),\n      ),\n";

/// Two packages, each with a module that imports `server-only` (not installed),
/// a module that does not, and a test of each. Their test scripts run vitest; a
/// CI job runs the scripts.
fn repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(
        r,
        "package.json",
        r#"{"name":"root","private":true,"workspaces":["packages/*"],"devDependencies":{"turbo":"2.0.0","vitest":"3.0.0"}}"#,
    );
    write(r, ".gitignore", ".turbo/trace/\n");
    write(r, "turbo.json", r#"{"tasks":{"test":{}}}"#);
    write(
        r,
        ".github/workflows/ci.yml",
        "on: [push]\njobs:\n  test:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: \
         actions/checkout@v4\n      - run: pnpm turbo run test\n",
    );
    for (pkg, config) in [
        ("a", with_aliases("", "node")),
        ("b", NO_ALIASES.to_owned()),
    ] {
        let p = |rel: &str| format!("packages/{pkg}/{rel}");
        write(
            r,
            &p("package.json"),
            &format!(
                r#"{{"name":"@acme/{pkg}","version":"1.0.0","scripts":{{"test":"vitest run"}}}}"#
            ),
        );
        write(r, &p("vitest.config.ts"), &config);
        write(
            r,
            &p("src/server.ts"),
            "import 'server-only';\n\nexport const secret = 1;\n",
        );
        write(r, &p("src/plain.ts"), "export const plain = 1;\n");
        write(r, &p("test/server-only-stub.ts"), "export {};\n");
        write(
            r,
            &p("test/server.test.ts"),
            "import { expect, test } from 'vitest';\nimport { secret } from \
             '../src/server';\n\ntest('secret', () => expect(secret).toBe(1));\n",
        );
        write(
            r,
            &p("test/plain.test.ts"),
            "import { expect, test } from 'vitest';\nimport { plain } from \
             '../src/plain';\n\ntest('plain', () => expect(plain).toBe(1));\n",
        );
    }
    git(r, &["init", "-q", "-b", "main"]);
    git(r, &["add", "-A"]);
    git(r, &["commit", "-qm", "base"]);
    d
}

const OFF: [(&str, &str); 1] = [("TURBO_TRACE_DEBUG_CONFIG_FIELDS", "0")];
const A_SERVER: &str = "packages/a/test/server.test.ts";
const A_PLAIN: &str = "packages/a/test/plain.test.ts";
const B_SERVER: &str = "packages/b/test/server.test.ts";
const B_PLAIN: &str = "packages/b/test/plain.test.ts";

#[test]
fn an_alias_entry_selects_the_tests_that_load_an_importer_of_its_request() {
    let d = repo();
    let r = d.path();
    // One more entry in an alias table: `server-only` now loads a stub. The test
    // that loads the module importing it can tell; the other test of the
    // package cannot.
    write(
        r,
        "packages/a/vitest.config.ts",
        &with_aliases(STUB_ENTRY, "node"),
    );
    assert_eq!(since(r, &OFF), [A_PLAIN, A_SERVER]);
    assert_eq!(since(r, &[]), [A_SERVER]);
    // The entry goes away again (the base revision has it now).
    git(r, &["commit", "-qam", "stub server-only in a"]);
    write(r, "packages/a/vitest.config.ts", &with_aliases("", "node"));
    assert_eq!(since(r, &[]), [A_SERVER]);
    // An alias for a request no module makes.
    write(
        r,
        "packages/a/vitest.config.ts",
        &with_aliases(
            &format!(
                "{STUB_ENTRY}      'client-only': fileURLToPath(new \
                 URL('./test/server-only-stub.ts', import.meta.url)),\n"
            ),
            "node",
        ),
    );
    assert_eq!(since(r, &[]), Vec::<String>::new());
}

#[test]
fn a_first_alias_block_is_an_alias_entry() {
    let d = repo();
    let r = d.path();
    // The config had no `resolve` at all: the edit adds the block and the import of
    // `node:url` its value needs.
    write(r, "packages/b/vitest.config.ts", STUB_BLOCK);
    assert_eq!(since(r, &OFF), [B_PLAIN, B_SERVER]);
    assert_eq!(since(r, &[]), [B_SERVER]);
}

#[test]
fn other_edits_of_the_config_select_its_tests() {
    let d = repo();
    let r = d.path();
    // Another field.
    write(r, "packages/a/vitest.config.ts", &with_aliases("", "jsdom"));
    assert_eq!(since(r, &[]), [A_PLAIN, A_SERVER]);
    // An alias entry and another field.
    write(
        r,
        "packages/a/vitest.config.ts",
        &with_aliases(STUB_ENTRY, "jsdom"),
    );
    assert_eq!(since(r, &[]), [A_PLAIN, A_SERVER]);
    // Code next to the config object.
    write(
        r,
        "packages/a/vitest.config.ts",
        &with_aliases(STUB_ENTRY, "node").replace(
            "export default",
            "process.env.TZ = 'UTC';\n\nexport default",
        ),
    );
    assert_eq!(since(r, &[]), [A_PLAIN, A_SERVER]);
    // A value the adapter cannot read.
    write(
        r,
        "packages/a/vitest.config.ts",
        &with_aliases("      ...loadAliases(),\n", "node"),
    );
    assert_eq!(since(r, &[]), [A_PLAIN, A_SERVER]);
    // Dependencies run through vite make requests the graph does not record: an
    // alias entry of such a config is a change for every test.
    git(r, &["checkout", "-q", "."]);
    let inlined = |config: &str| {
        config.replace(
            "globals: true,",
            "globals: true,\n    server: { deps: { inline: ['some-dep'] } },",
        )
    };
    write(r, "packages/b/vitest.config.ts", &inlined(NO_ALIASES));
    git(r, &["commit", "-qam", "b inlines a dependency"]);
    write(r, "packages/b/vitest.config.ts", &inlined(STUB_BLOCK));
    assert_eq!(since(r, &[]), [B_PLAIN, B_SERVER]);
}
