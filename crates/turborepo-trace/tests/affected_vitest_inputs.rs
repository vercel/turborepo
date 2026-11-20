//! `affected` end to end for what a vitest test reads without an import
//! statement saying so: the modules of an `import.meta.glob`, the file of a
//! `toMatchFileSnapshot`, and the env files vite loads for `import.meta.env`.
// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{path::Path, process::Command};

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

/// The output lines of `harness affected <args>` in `root`.
fn run(root: &Path, args: &[&str]) -> Vec<String> {
    let mut c = Command::new(support::harness());
    c.args(["affected", "--tests", "all"])
        .args(args)
        .current_dir(root);
    let o = c.output().expect("run the harness");
    assert!(
        o.status.success(),
        "turbotrace affected {args:?} failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// The tests a change of `changed` selects.
fn affected(root: &Path, changed: &str) -> Vec<String> {
    let mut out = run(root, &["--changed", changed]);
    out.sort();
    out
}

/// Whether the selection document for a change of `changed` lists `file` as a
/// test that runs: selected, or an unknown test (which always runs).
fn runs_it(root: &Path, changed: &str, file: &str) -> bool {
    let doc = run(root, &["--format", "selection", "--changed", changed]).join("\n");
    let quoted = format!("\"{file}\"");
    ["\"selected\": [", "\"unknown\": ["].iter().any(|key| {
        doc.match_indices(key).any(|(i, _)| {
            // The list, to the bracket that closes it.
            let rest = &doc[i + key.len()..];
            let mut depth = 1;
            let end = rest
                .char_indices()
                .find(|(_, c)| {
                    match c {
                        '[' => depth += 1,
                        ']' => depth -= 1,
                        _ => {}
                    }
                    depth == 0
                })
                .map_or(rest.len(), |(at, _)| at);
            rest[..end].contains(&quoted)
        })
    })
}

/// ```text
/// package.json            `test`: vitest run
/// vitest.config.ts        the config under test (none: a run with the defaults)
/// .env, .env.test, .env.production
/// src/routes.ts           import.meta.glob('./pages/*.ts')
/// src/pages/a.ts, b.ts
/// test/routes.test.ts     imports src/routes
/// test/snap.test.ts       toMatchFileSnapshot('./out/expected.txt')
/// test/out/expected.txt
/// test/plain.test.ts      reads nothing
/// ```
fn repo(config: Option<&str>) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(
        r,
        "package.json",
        r#"{"name":"r","version":"1.0.0","private":true,"scripts":{"test":"vitest run"},"devDependencies":{"vitest":"^4.0.18"}}"#,
    );
    if let Some(config) = config {
        write(r, "vitest.config.ts", config);
    }
    for (name, value) in [(".env", "A"), (".env.test", "B"), (".env.production", "C")] {
        write(r, name, &format!("VITE_NAME={value}\n"));
    }
    write(r, "envs/.env", "VITE_NAME=D\n");
    write(
        r,
        "src/routes.ts",
        "export const pages = import.meta.glob('./pages/*.ts');\n",
    );
    write(r, "src/pages/a.ts", "export const a = 1;\n");
    write(r, "src/pages/b.ts", "export const b = 2;\n");
    write(
        r,
        "test/routes.test.ts",
        "import { expect, test } from 'vitest';\nimport { pages } from \
         '../src/routes';\ntest('pages', () => {\n  \
         expect(Object.keys(pages)).toHaveLength(2);\n});\n",
    );
    write(
        r,
        "test/snap.test.ts",
        "import { expect, test } from 'vitest';\ntest('snap', async () => {\n  await \
         expect('x').toMatchFileSnapshot('./out/expected.txt');\n});\n",
    );
    write(r, "test/out/expected.txt", "x");
    write(
        r,
        "test/plain.test.ts",
        "import { expect, test } from 'vitest';\ntest('plain', () => {\n  \
         expect(1).toBe(1);\n});\n",
    );
    d
}

const EMPTY: &str = "export default {};\n";
const ROUTES: &str = "test/routes.test.ts";
const SNAP: &str = "test/snap.test.ts";
const PLAIN: &str = "test/plain.test.ts";

/// The modules an `import.meta.glob` matches are loaded by the file that holds
/// it: a change of one selects the tests that load that file, and so does a
/// new file the pattern takes.
#[test]
fn a_glob_import_loads_the_modules_it_matches() {
    let d = repo(Some(EMPTY));
    let r = d.path();
    assert_eq!(affected(r, "src/pages/a.ts"), [ROUTES]);
    write(r, "src/pages/c.ts", "export const c = 3;\n");
    assert_eq!(affected(r, "src/pages/c.ts"), [ROUTES]);
    // A file the pattern does not take.
    write(r, "src/pages/notes.md", "notes\n");
    assert_eq!(affected(r, "src/pages/notes.md"), Vec::<String>::new());
}

/// Several patterns, a pattern below a directory of any depth, and one that
/// leaves files out (which takes nothing away here).
#[test]
fn glob_imports_with_several_patterns() {
    let d = repo(Some(EMPTY));
    let r = d.path();
    write(
        r,
        "src/routes.ts",
        "export const pages = import.meta.glob(['./pages/**/*.ts', '../data/*.json', \
         '!./pages/b.ts'], { eager: true });\n",
    );
    write(r, "src/pages/deep/x.ts", "export const x = 1;\n");
    write(r, "data/table.json", "{}\n");
    for changed in ["src/pages/deep/x.ts", "data/table.json", "src/pages/b.ts"] {
        assert_eq!(affected(r, changed), [ROUTES], "{changed}");
    }
}

/// A pattern that is computed, or that starts at the project root, is a load
/// nothing bounds: the tests that load the file are selected by any change.
#[test]
fn a_glob_import_that_is_not_a_literal_relative_pattern_is_open() {
    for pattern in ["dir + '/*.ts'", "'/src/pages/*.ts'"] {
        let d = repo(Some(EMPTY));
        let r = d.path();
        write(
            r,
            "src/routes.ts",
            &format!("const dir = './pages';\nexport const pages = import.meta.glob({pattern});\n"),
        );
        assert!(runs_it(r, "src/pages/a.ts", ROUTES), "{pattern}");
    }
}

/// With the option `base`, the patterns are relative to that directory: one
/// relative to the file is followed, any other (from the project root,
/// computed, or in options that are not written in the call) is a load
/// nothing bounds.
#[test]
fn a_glob_import_with_a_base_is_relative_to_the_base() {
    let d = repo(Some(EMPTY));
    let r = d.path();
    write(r, "data/table.json", "{}\n");
    write(r, "src/b.ts", "export const here = 1;\n");
    for (call, changed) in [
        (
            "import.meta.glob('./*.ts', { base: './pages' })",
            "src/pages/a.ts",
        ),
        (
            "import.meta.glob(['./*.json'], { eager: true, base: '../data', })",
            "data/table.json",
        ),
        (
            "import.meta.glob('../*.ts', { 'base': './pages/' })",
            "src/b.ts",
        ),
    ] {
        write(
            r,
            "src/routes.ts",
            &format!("export const pages = {call};\n"),
        );
        assert_eq!(affected(r, changed), [ROUTES], "{call}");
    }
    // The pattern does not start at the file any more.
    write(
        r,
        "src/routes.ts",
        "export const pages = import.meta.glob('./*.ts', { base: './pages' });\n",
    );
    assert_eq!(affected(r, "src/b.ts"), Vec::<String>::new());
    for call in [
        "import.meta.glob('./*.ts', { base: '/src/pages' })",
        "import.meta.glob('./*.ts', { base: dir })",
        "import.meta.glob('./*.ts', { base })",
        "import.meta.glob('./*.ts', { ...options })",
        "import.meta.glob('./*.ts', options)",
    ] {
        write(
            r,
            "src/routes.ts",
            &format!(
                "const dir = './pages';\nconst base = dir;\nconst options = {{ base }};\nexport \
                 const pages = {call};\n"
            ),
        );
        assert!(runs_it(r, "src/pages/a.ts", ROUTES), "{call}");
    }
}

/// A glob import loads every file its pattern matches, also the kinds a
/// traced `require` leaves out: a `README.md`, a C source, a declaration file.
#[test]
fn a_glob_import_loads_files_of_any_kind() {
    let d = repo(Some(EMPTY));
    let r = d.path();
    write(
        r,
        "src/routes.ts",
        "export const docs = import.meta.glob('./pages/*.md', { query: '?raw' });\nexport const \
         all = import.meta.glob('./native/*', { query: '?raw' });\n",
    );
    for file in [
        "src/pages/README.md",
        "src/pages/CHANGELOG.md",
        "src/native/add.c",
        "src/native/add.h",
        "src/native/types.d.ts",
    ] {
        write(r, file, "text\n");
        assert_eq!(affected(r, file), [ROUTES], "{file}");
    }
}

/// The file of a `toMatchFileSnapshot` is an input of the test, also before
/// it is written.
#[test]
fn a_file_snapshot_is_an_input_of_its_test() {
    let d = repo(Some(EMPTY));
    let r = d.path();
    assert_eq!(affected(r, "test/out/expected.txt"), [SNAP]);
    std::fs::remove_file(r.join("test/out/expected.txt")).unwrap();
    assert_eq!(affected(r, "test/out/expected.txt"), [SNAP]);
}

/// The env files vite loads for a test run are inputs of every test of the
/// project: `.env`, `.env.local`, `.env.test` and `.env.test.local` in the
/// root, not the files of another mode.
#[test]
fn env_files_are_inputs_of_the_tests_of_the_project() {
    let d = repo(Some(EMPTY));
    let r = d.path();
    for changed in [".env", ".env.test"] {
        assert_eq!(affected(r, changed), [PLAIN, ROUTES, SNAP], "{changed}");
    }
    // One that is added.
    write(r, ".env.local", "VITE_NAME=L\n");
    assert_eq!(affected(r, ".env.local"), [PLAIN, ROUTES, SNAP]);
    assert_eq!(affected(r, ".env.production"), Vec::<String>::new());
    assert_eq!(affected(r, "envs/.env"), Vec::<String>::new());
}

/// `envDir` names the directory of the env files; `false` loads none.
#[test]
fn env_dir_names_where_the_env_files_are() {
    let d = repo(Some("export default { envDir: 'envs' };\n"));
    let r = d.path();
    assert_eq!(affected(r, "envs/.env"), [PLAIN, ROUTES, SNAP]);
    assert_eq!(affected(r, ".env"), Vec::<String>::new());
    let d = repo(Some("export default { envDir: false };\n"));
    assert_eq!(affected(d.path(), ".env"), Vec::<String>::new());
}

/// A test of a run without a config always runs, whatever changes.
#[test]
fn a_test_without_a_config_always_runs() {
    let d = repo(None);
    let r = d.path();
    for changed in ["src/pages/a.ts", "test/out/expected.txt", ".env"] {
        for test in [ROUTES, SNAP, PLAIN] {
            assert!(runs_it(r, changed, test), "{changed} {test}");
        }
    }
}
