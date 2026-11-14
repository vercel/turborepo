//! Entry specs that name a file whose path has glob metacharacters.
//!
//! The shape comes from a real app: Next.js route directories are called
//! `[teamSlug]`, `(dashboard)` and `~`, and tests live next to the components
//! below them. `[teamSlug]` is a character class to the glob matcher, so the
//! path of such a test, taken as a pattern, matched no file: `affected
//! --explain <path>` without `--tests` then printed `unaffected` for a test
//! that a changed file reaches through two plain imports.
// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{path::Path, process::Command};

const DIR: &str = "apps/site/app/(dashboard)/[teamSlug]/(team)/~/keys/_components";

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
        r#"{"name":"site","version":"1.0.0","scripts":{"test":"jest"}}"#,
    );
    write(r, "apps/site/jest.config.js", "module.exports = {};\n");
    write(
        r,
        &format!("{DIR}/expiration-field.tsx"),
        "export function ExpirationField(props: { value: string }) {\n  return <input \
         value={props.value} />;\n}\n",
    );
    write(
        r,
        &format!("{DIR}/create-key.tsx"),
        "import { ExpirationField } from './expiration-field';\n\nexport function CreateKey() {\n  return <ExpirationField value=\"never\" />;\n}\n",
    );
    write(
        r,
        &format!("{DIR}/create-key.test.tsx"),
        "import { CreateKey } from './create-key';\n\ntest('renders', () => {\n  \
         expect(CreateKey).toBeDefined();\n});\n",
    );
    // An unrelated module and its test in the same directory.
    write(r, &format!("{DIR}/other.ts"), "export const other = 1;\n");
    write(
        r,
        &format!("{DIR}/other.test.ts"),
        "import { other } from './other';\n\ntest('other', () => {\n  \
         expect(other).toBe(1);\n});\n",
    );
    // Directories a character class does match.
    write(r, "apps/site/lib/a/x.ts", "export const x = 1;\n");
    write(r, "apps/site/lib/b/x.ts", "export const x = 2;\n");
    d
}

fn run(root: &Path, args: &[&str]) -> String {
    let o = Command::new(support::harness())
        .arg("affected")
        .args(args)
        .current_dir(root)
        .output()
        .expect("run the harness");
    assert!(
        o.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn explain_takes_a_path_with_brackets_as_the_file() {
    let d = repo();
    let test = format!("{DIR}/create-key.test.tsx");
    let changed = format!("{DIR}/expiration-field.tsx");
    // Without `--tests` (the explained file is an entry by itself) and with it.
    for tests in [&[][..], &["--tests", "jest"][..]] {
        for policy in [&[][..], &["--on-uncertain", "ignore"][..]] {
            let mut args = vec!["--changed", changed.as_str(), "--explain", test.as_str()];
            args.extend_from_slice(tests);
            args.extend_from_slice(policy);
            let out = run(d.path(), &args);
            let first = out.lines().next().unwrap_or("");
            assert!(
                first.starts_with(&format!("affected {test}: modified {changed}")),
                "{args:?}: {out}"
            );
            assert!(first.contains("create-key.tsx"), "{args:?}: {out}");
        }
    }
}

#[test]
fn an_unrelated_edit_leaves_the_bracket_path_unaffected() {
    let d = repo();
    let test = format!("{DIR}/create-key.test.tsx");
    let changed = format!("{DIR}/other.ts");
    for tests in [&[][..], &["--tests", "jest"][..]] {
        let mut args = vec![
            "--changed",
            changed.as_str(),
            "--explain",
            test.as_str(),
            "--on-uncertain",
            "ignore",
        ];
        args.extend_from_slice(tests);
        let out = run(d.path(), &args);
        assert_eq!(
            out.lines().next(),
            Some(format!("unaffected {test}").as_str()),
            "{args:?}: {out}"
        );
    }
}

#[test]
fn a_positional_entry_with_brackets_is_the_file() {
    let d = repo();
    let test = format!("{DIR}/create-key.test.tsx");
    let changed = format!("{DIR}/expiration-field.tsx");
    let out = run(
        d.path(),
        &[
            "--changed",
            changed.as_str(),
            "--format",
            "list",
            "--on-uncertain",
            "ignore",
            test.as_str(),
        ],
    );
    assert_eq!(out.lines().collect::<Vec<_>>(), [test.as_str()], "{out}");
}

#[test]
fn patterns_stay_patterns() {
    let d = repo();
    // A class that names no file is a pattern: it selects the directories it
    // matches.
    let out = run(
        d.path(),
        &[
            "--changed",
            "apps/site/lib/a/x.ts",
            "--format",
            "list",
            "--on-uncertain",
            "ignore",
            "apps/site/lib/[ab]/x.ts",
        ],
    );
    assert_eq!(
        out.lines().collect::<Vec<_>>(),
        ["apps/site/lib/a/x.ts"],
        "{out}"
    );
    // `**` over the route directories still finds the test.
    let changed = format!("{DIR}/expiration-field.tsx");
    let out = run(
        d.path(),
        &[
            "--changed",
            changed.as_str(),
            "--format",
            "list",
            "--on-uncertain",
            "ignore",
            "apps/site/app/**/*.test.tsx",
        ],
    );
    assert_eq!(
        out.lines().collect::<Vec<_>>(),
        [format!("{DIR}/create-key.test.tsx").as_str()],
        "{out}"
    );
}

#[test]
fn a_pattern_that_matches_nothing_has_no_verdict() {
    let d = repo();
    // The path of a file that does not exist, with a class: a pattern without
    // matches. It is reported as no entry, never as `unaffected`.
    let spec = format!("{DIR}/[missing]/gone.test.tsx");
    let changed = format!("{DIR}/expiration-field.tsx");
    let out = run(
        d.path(),
        &[
            "--changed",
            changed.as_str(),
            "--explain",
            spec.as_str(),
            "--on-uncertain",
            "ignore",
        ],
    );
    assert_eq!(
        out.lines().next(),
        Some(format!("{spec}: not an entry in the graph").as_str()),
        "{out}"
    );
}
