//! `turbotrace affected --since`: a dependency edit in a `package.json` selects
//! the consumers of that dependency, including the ones that use it without an
//! import (the JSX runtime, a preset named in a config string, a binary run by
//! a script, `@types`). A dependency nobody in the package references selects
//! nothing and is reported.
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

fn ok_env(root: &Path, args: &[&str], env: &[(&str, &str)]) -> String {
    let o = run_env(root, args, env);
    assert!(
        o.status.success(),
        "turbotrace {args:?} failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout).unwrap()
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

/// The dependencies of `packages/ui` at the base revision: `(name, version)`.
const BASE: [(&str, &str); 9] = [
    ("@types/used-lib", "1.0.0"),
    ("babel-preset-fancy", "1.0.0"),
    ("cli-tool", "1.0.0"),
    ("fancy-jsx", "1.0.0"),
    ("gone-lib", "1.0.0"),
    ("macros-thing", "1.0.0"),
    ("react", "18.0.0"),
    ("unreferenced", "1.0.0"),
    ("used-lib", "1.0.0"),
];

fn manifest(deps: &[(&str, &str)]) -> String {
    let list: Vec<String> = deps
        .iter()
        .map(|(n, v)| format!("\"{n}\": \"{v}\""))
        .collect();
    format!(
        "{{\n  \"name\": \"@acme/ui\",\n  \"version\": \"1.0.0\",\n  \"scripts\": {{ \"deploy\": \
         \"cli-tool deploy --prod\", \"lint\": \"acme-lint\" }},\n  \"dependencies\": {{ {} \
         }}\n}}\n",
        list.join(", ")
    )
}

fn lock(deps: &[(&str, &str)]) -> String {
    let mut s = String::from(
        "lockfileVersion: '9.0'\n\nsettings:\n  autoInstallPeers: false\n  \
         excludeLinksFromLockfile: false\n\nimporters:\n\n  .: {}\n\n  packages/ui:\n    \
         dependencies:\n",
    );
    for (n, v) in deps {
        let key = if n.starts_with('@') {
            format!("'{n}'")
        } else {
            (*n).to_owned()
        };
        s.push_str(&format!(
            "      {key}:\n        specifier: {v}\n        version: {v}\n"
        ));
    }
    s.push_str("\npackages:\n");
    for (n, v) in deps {
        let key = if n.starts_with('@') {
            format!("'{n}@{v}'")
        } else {
            format!("{n}@{v}")
        };
        s.push_str(&format!(
            "\n  {key}:\n    resolution: {{integrity: sha512-{}-{v}==}}\n",
            n.replace(['@', '/'], "")
        ));
    }
    s.push_str("\nsnapshots:\n");
    for (n, v) in deps {
        let key = if n.starts_with('@') {
            format!("'{n}@{v}'")
        } else {
            format!("{n}@{v}")
        };
        s.push_str(&format!("\n  {key}: {{}}\n"));
    }
    s
}

const BUTTON: &str = "packages/ui/test/button.test.tsx";
const USES: &str = "packages/ui/test/uses.test.ts";
const PLAIN: &str = "packages/ui/test/plain.test.ts";
const BABEL: &str = "packages/ui/babel.config.js";
const ENTRIES: [&str; 4] = [BUTTON, USES, PLAIN, BABEL];

/// `@acme/ui`: `button.tsx` is JSX without an import of `react` (the automatic
/// runtime), `uses.ts` imports `used-lib`, `plain.ts` nothing;
/// `babel.config.js` names `babel-preset-fancy` by its short name; the `deploy`
/// script runs the binary of `cli-tool`; `tsconfig.json` sets
/// `jsxImportSource`; `.babelrc` (not a source file) names `macros-thing`.
/// `gone-lib` and `unreferenced` are named nowhere.
fn repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"root","private":true}"#);
    write(r, "pnpm-workspace.yaml", "packages:\n  - 'packages/*'\n");
    write(r, ".gitignore", ".turbo/trace/\n");
    write(r, "packages/ui/package.json", &manifest(&BASE));
    write(r, "pnpm-lock.yaml", &lock(&BASE));
    write(
        r,
        "packages/ui/tsconfig.json",
        r#"{"compilerOptions":{"jsx":"react-jsx","jsxImportSource":"fancy-jsx"},"include":["src","test"]}"#,
    );
    write(r, BABEL, "module.exports = { presets: ['fancy'] };\n");
    write(r, "packages/ui/.babelrc", r#"{"plugins":["macros-thing"]}"#);
    write(
        r,
        "packages/ui/src/button.tsx",
        "export const Button = () => <button>ok</button>;\n",
    );
    write(
        r,
        "packages/ui/src/uses.ts",
        "import used from 'used-lib';\nexport const u = used;\n",
    );
    write(r, "packages/ui/src/plain.ts", "export const p = 1;\n");
    write(
        r,
        BUTTON,
        "import { Button } from '../src/button';\ntest('b', () => expect(Button).toBeTruthy());\n",
    );
    write(
        r,
        USES,
        "import { u } from '../src/uses';\ntest('u', () => expect(u).toBeTruthy());\n",
    );
    write(
        r,
        PLAIN,
        "import { p } from '../src/plain';\ntest('p', () => expect(p).toBe(1));\n",
    );
    git(r, &["init", "-q"]);
    git(r, &["add", "-A"]);
    git(r, &["commit", "-q", "-m", "base"]);
    d
}

/// Writes the manifest and the lockfile with `name` at `version` (`None`:
/// removed).
fn set(r: &Path, name: &str, version: Option<&str>) {
    let mut deps: Vec<(&str, &str)> = BASE.iter().copied().filter(|(n, _)| *n != name).collect();
    if let Some(v) = version {
        deps.push((name, v));
        deps.sort();
    }
    write(r, "packages/ui/package.json", &manifest(&deps));
    write(r, "pnpm-lock.yaml", &lock(&deps));
}

fn since_env(r: &Path, env: &[(&str, &str)]) -> Vec<String> {
    let mut args = vec!["affected", "--since", "HEAD", "--on-uncertain", "ignore"];
    args.extend(ENTRIES);
    ok_env(r, &args, env).lines().map(str::to_owned).collect()
}

fn since(r: &Path) -> Vec<String> {
    since_env(r, &[])
}

fn tasks(r: &Path) -> Vec<String> {
    ok_env(
        r,
        &[
            "affected",
            "--since",
            "HEAD",
            "--on-uncertain",
            "ignore",
            "--tasks",
            "**",
        ],
        &[],
    )
    .lines()
    .map(str::to_owned)
    .collect()
}

fn stats(r: &Path) -> String {
    let mut args = vec![
        "affected",
        "--since",
        "HEAD",
        "--on-uncertain",
        "ignore",
        "--stats",
    ];
    args.extend(ENTRIES);
    String::from_utf8_lossy(&run_env(r, &args, &[]).stderr).into_owned()
}

/// Dependency edits as whole-file changes.
const NODEPS: [(&str, &str); 1] = [("TURBO_TRACE_DEBUG_CONFIG_FIELDS", "nodeps")];

#[test]
fn consumers_without_an_import() {
    let d = repo();
    let r = d.path();
    // The JSX runtime: `react` is used by every JSX file, with no import to show
    // it.
    set(r, "react", Some("18.1.0"));
    assert_eq!(since_env(r, &NODEPS), [BABEL, BUTTON, PLAIN, USES]);
    assert_eq!(since(r), [BUTTON]);
    // The package a config names as `jsxImportSource`.
    set(r, "fancy-jsx", Some("1.1.0"));
    assert_eq!(since(r), [BUTTON]);
    // An imported package: its importer.
    set(r, "used-lib", Some("1.1.0"));
    assert_eq!(since(r), [USES]);
    // A binary run by a script: the script's task.
    set(r, "cli-tool", Some("1.1.0"));
    assert!(since(r).is_empty());
    assert_eq!(tasks(r), ["task:packages/ui#deploy"]);
    // `@types/<n>`: type checking of the files that link `<n>`.
    set(r, "@types/used-lib", Some("1.1.0"));
    assert_eq!(since(r), [USES]);
}

#[test]
fn a_dependency_nobody_references_selects_nothing() {
    let d = repo();
    let r = d.path();
    // Removed, and nothing in the package refers to it.
    set(r, "gone-lib", None);
    assert_eq!(since_env(r, &NODEPS), [BABEL, BUTTON, PLAIN, USES]);
    assert!(since(r).is_empty());
    let err = stats(r);
    assert!(
        err.contains("none found for gone-lib (not linked, probed or named in the package"),
        "{err}"
    );
    // Bumped or added with the lockfile: what the install changes is the lockfile
    // diff's to say (a peer of another dependency); no file of the package
    // names it.
    set(r, "unreferenced", Some("2.0.0"));
    assert!(since(r).is_empty());
    set(r, "brand-new", Some("1.0.0"));
    assert!(since(r).is_empty());
    let mut args = vec![
        "affected",
        "--since",
        "HEAD",
        "--on-uncertain",
        "ignore",
        "--format",
        "json",
    ];
    args.extend(ENTRIES);
    let v: serde_json::Value = serde_json::from_str(&ok_env(r, &args, &[])).unwrap();
    let c = &v["configs"][0];
    assert_eq!(c["field_level"], true);
    assert_eq!(c["unconsumed"], serde_json::json!(["brand-new"]));

    // Removed while a file still imports it: that file.
    set(r, "used-lib", None);
    assert_eq!(since(r), [USES]);
}

#[test]
fn unknown_consumers_keep_the_whole_file() {
    let d = repo();
    let r = d.path();
    // Named by a tool's config (`.babelrc`): the tool applies to files that have no
    // edge to its config, so who consumes the plugin is not known.
    set(r, "macros-thing", Some("1.1.0"));
    assert_eq!(since(r), [BABEL, BUTTON, PLAIN, USES]);
    let err = stats(r);
    assert!(
        err.contains(
            "whole-file change: changed dependenc(ies) whose consumers are not known: macros-thing"
        ),
        "{err}"
    );
    // The same for a preset under the short name babel accepts (`fancy` in
    // `babel.config.js` is `babel-preset-fancy`).
    set(r, "babel-preset-fancy", Some("1.1.0"));
    assert_eq!(since(r), [BABEL, BUTTON, PLAIN, USES]);
    // A config that imports the package is a linker like any file: only it is
    // seeded.
    git(r, &["checkout", "-q", "--", "."]);
    write(
        r,
        BABEL,
        "const used = require('used-lib');\nmodule.exports = { presets: ['fancy'], used };\n",
    );
    git(r, &["add", "-A"]);
    git(r, &["commit", "-q", "-m", "config imports"]);
    set(r, "used-lib", Some("1.2.0"));
    assert_eq!(since(r), [BABEL, USES]);
    // A range edit without a lockfile change: nothing says what an install would
    // do.
    git(r, &["checkout", "-q", "--", "."]);
    let mut deps = BASE.to_vec();
    deps.retain(|(n, _)| *n != "unreferenced");
    deps.push(("unreferenced", "^1.2.0"));
    deps.sort();
    write(r, "packages/ui/package.json", &manifest(&deps));
    assert_eq!(since(r), [BABEL, BUTTON, PLAIN, USES]);
}
