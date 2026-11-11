//! `affected --since`: the resolution fields of a `package.json`
//! (`exports`, `imports`, `main`, `types`, `typesVersions`, ...) are applied
//! entry by entry. A changed, added or removed entry selects the importers
//! whose requests it answers, and nothing else; shapes outside the model keep
//! the whole-file treatment.
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
    c.output().expect("run the harness")
}

fn ok_env(root: &Path, args: &[&str], env: &[(&str, &str)]) -> String {
    let o = run_env(root, args, env);
    assert!(
        o.status.success(),
        "harness {args:?} failed: {}",
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

const LIB: &str = r##"{
  "name": "@acme/lib",
  "version": "1.0.0",
  "main": "./src/index.js",
  "types": "./src/index.d.ts",
  "exports": {
    ".": { "require": "./src/index.js", "default": "./src/index.js" },
    "./util": { "types": "./src/util.d.ts", "default": "./src/util.js" },
    "./feature/*": "./src/feature/*.js",
    "./internal/*": null
  },
  "imports": { "#cfg": "./src/cfg.js", "#gen/*": "./src/gen/*.js" },
  "typesVersions": { "*": { "util": ["src/util.d.ts"] } }
}
"##;

// The tests of the importing package, one per request it makes of `@acme/lib`.
const ROOT: &str = "packages/app/test/root.test.js";
const UTIL: &str = "packages/app/test/util.test.js";
const FEAT: &str = "packages/app/test/feat.test.js";
const EXTRA: &str = "packages/app/test/extra.test.js";
const NONE: &str = "packages/app/test/none.test.js";
const VEND: &str = "packages/app/test/vend.test.js";
// The tests of the package itself.
const OWN_CFG: &str = "packages/lib/test/own-cfg.test.js";
const OWN_GEN: &str = "packages/lib/test/own-gen.test.js";
const PLAIN: &str = "packages/lib/test/plain.test.js";
const ENTRIES: [&str; 9] = [ROOT, UTIL, FEAT, EXTRA, NONE, VEND, OWN_CFG, OWN_GEN, PLAIN];

/// `@acme/lib` exports its root, `./util`, a `./feature/*` pattern and excludes
/// `./internal/*`; its own files use `#cfg` and `#gen/*`. `@acme/app` requires
/// the root, the subpath, a pattern match and `@acme/lib/extra`, which is not
/// exported; one of its files requires a vendored directory with its own
/// manifest.
fn repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"root","private":true}"#);
    write(r, "pnpm-workspace.yaml", "packages:\n  - 'packages/*'\n");
    write(r, ".gitignore", ".turbo/trace/\n");
    write(r, "packages/lib/package.json", LIB);
    for f in [
        "index",
        "alt",
        "util",
        "util2",
        "extra",
        "cfg",
        "cfg2",
        "plain",
        "feature/a",
        "feature/b",
        "feat2/a",
        "gen/one",
        "gen2/one",
        "internal/secret",
    ] {
        write(
            r,
            &format!("packages/lib/src/{f}.js"),
            &format!("module.exports = '{f}';\n"),
        );
    }
    write(
        r,
        "packages/lib/src/own-cfg.js",
        "module.exports = require('#cfg');\n",
    );
    write(
        r,
        "packages/lib/src/own-gen.js",
        "module.exports = require('#gen/one');\n",
    );
    for (t, src) in [(OWN_CFG, "own-cfg"), (OWN_GEN, "own-gen"), (PLAIN, "plain")] {
        write(
            r,
            t,
            &format!("require('../src/{src}');\ntest('t', () => {{}});\n"),
        );
    }
    write(
        r,
        "packages/app/package.json",
        r#"{"name":"@acme/app","version":"1.0.0","dependencies":{"@acme/lib":"workspace:*"}}"#,
    );
    std::fs::create_dir_all(r.join("packages/app/node_modules/@acme")).unwrap();
    #[cfg(unix)]
    turborepo_trace_core::os::symlink(
        "../../../lib",
        r.join("packages/app/node_modules/@acme/lib"),
    )
    .unwrap();
    write(
        r,
        "packages/app/jest.config.js",
        "module.exports = { testMatch: ['**/test/**/*.test.js'] };\n",
    );
    write(
        r,
        "packages/app/vendor/pkg/package.json",
        r#"{"main":"a.js"}"#,
    );
    write(r, "packages/app/vendor/pkg/a.js", "module.exports = 'a';\n");
    write(r, "packages/app/vendor/pkg/b.js", "module.exports = 'b';\n");
    for (name, spec) in [
        ("root", "@acme/lib"),
        ("util", "@acme/lib/util"),
        ("feat", "@acme/lib/feature/a"),
        ("extra", "@acme/lib/extra"),
        ("vend", "../vendor/pkg"),
    ] {
        write(
            r,
            &format!("packages/app/src/{name}.js"),
            &format!("module.exports = require('{spec}');\n"),
        );
    }
    write(r, "packages/app/src/none.js", "module.exports = 0;\n");
    for (t, src) in [
        (ROOT, "root"),
        (UTIL, "util"),
        (FEAT, "feat"),
        (EXTRA, "extra"),
        (NONE, "none"),
        (VEND, "vend"),
    ] {
        write(
            r,
            t,
            &format!("require('../src/{src}');\ntest('t', () => {{}});\n"),
        );
    }
    git(r, &["init", "-q"]);
    git(r, &["add", "-A"]);
    git(r, &["commit", "-q", "-m", "base"]);
    d
}

/// The affected entries among [`ENTRIES`] against `HEAD`, ignoring uncertainty.
fn since_env(r: &Path, env: &[(&str, &str)]) -> Vec<String> {
    let mut args = vec!["affected", "--since", "HEAD", "--on-uncertain", "ignore"];
    args.extend(ENTRIES);
    ok_env(r, &args, env).lines().map(str::to_owned).collect()
}

fn since(r: &Path) -> Vec<String> {
    since_env(r, &[])
}

/// Replaces `from` by `to` in the library's manifest (starting from the
/// committed text).
fn edit(r: &Path, from: &str, to: &str) {
    assert!(LIB.contains(from), "{from}");
    write(r, "packages/lib/package.json", &LIB.replace(from, to));
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

const OFF: [(&str, &str); 1] = [("TURBO_TRACE_DEBUG_CONFIG_FIELDS", "0")];
/// The plain file of the importing package that requests `./util` (not a test:
/// every edge counts for it, type-only ones included).
const UTIL_SRC: &str = "packages/app/src/util.js";
const ROOT_SRC: &str = "packages/app/src/root.js";
const FEAT_SRC: &str = "packages/app/src/feat.js";

#[test]
fn a_changed_entry_selects_the_importers_that_request_it() {
    let d = repo();
    let r = d.path();
    // An exact subpath: its importer, not the importers of the root or of other
    // subpaths, and not the package's own files.
    edit(
        r,
        r#""default": "./src/util.js""#,
        r#""default": "./src/util2.js""#,
    );
    // (As a whole-file change: everything that asked the manifest, the refused
    // request included.)
    assert_eq!(
        since_env(r, &OFF),
        [EXTRA, FEAT, ROOT, UTIL, OWN_CFG, OWN_GEN, PLAIN]
    );
    assert_eq!(since(r), [UTIL]);
    let err = stats(r);
    assert!(
        err.contains(
            "config packages/lib/package.json (exports): resolution fields applied per request of \
             each importer"
        ),
        "{err}"
    );
    assert!(
        err.contains("1 importer(s) (+0 for type-checking entries only)"),
        "{err}"
    );

    // A `null` target takes the subpath away.
    edit(
        r,
        r#""./util": { "types": "./src/util.d.ts", "default": "./src/util.js" }"#,
        r#""./util": null"#,
    );
    assert_eq!(since(r), [UTIL]);
    // An exclusion that nobody requests.
    edit(
        r,
        r#""./internal/*": null"#,
        r#""./internal/*": "./src/internal/*.js""#,
    );
    assert!(since(r).is_empty());

    // An added subpath captures the request that failed to resolve before.
    edit(
        r,
        r#""./internal/*": null"#,
        r#""./internal/*": null, "./extra": "./src/extra.js""#,
    );
    assert_eq!(since(r), [EXTRA]);
    // One that nobody requests.
    edit(
        r,
        r#""./internal/*": null"#,
        r#""./internal/*": null, "./unused": "./src/alt.js""#,
    );
    assert!(since(r).is_empty());
}

#[test]
fn patterns_conditions_and_entry_fields() {
    let d = repo();
    let r = d.path();
    // A changed pattern target: the requests the pattern answers.
    edit(
        r,
        r#""./feature/*": "./src/feature/*.js""#,
        r#""./feature/*": "./src/feat2/*.js""#,
    );
    assert_eq!(since(r), [FEAT]);
    // An added pattern captures the unexported request.
    edit(
        r,
        r#""./internal/*": null"#,
        r#""./internal/*": null, "./*": "./src/*.js""#,
    );
    assert_eq!(since(r), [EXTRA]);
    // A more specific pattern takes over what it matches.
    edit(
        r,
        r#""./internal/*": null"#,
        r#""./internal/*": null, "./feature/a": "./src/feat2/a.js""#,
    );
    assert_eq!(since(r), [FEAT]);
    // (`*` matches at least one character: `./feature/a*` does not answer
    // `./feature/a`.)
    edit(
        r,
        r#""./internal/*": null"#,
        r#""./internal/*": null, "./feature/a*": "./src/feat2/a*.js""#,
    );
    assert!(since(r).is_empty());

    // The order of conditions decides which one matches.
    edit(
        r,
        r#"{ "require": "./src/index.js", "default": "./src/index.js" }"#,
        r#"{ "default": "./src/index.js", "require": "./src/index.js" }"#,
    );
    assert_eq!(since(r), [ROOT]);
    // Nested conditions.
    edit(
        r,
        r#"{ "require": "./src/index.js", "default": "./src/index.js" }"#,
        r#"{ "require": { "node": "./src/index.js", "default": "./src/alt.js" }, "default": "./src/index.js" }"#,
    );
    assert_eq!(since(r), [ROOT]);
    // `main` is the root request.
    edit(
        r,
        r#""main": "./src/index.js""#,
        r#""main": "./src/alt.js""#,
    );
    assert_eq!(since(r), [ROOT]);

    // The manifest of a directory that a relative specifier names.
    git(r, &["checkout", "-q", "--", "."]);
    write(
        r,
        "packages/app/vendor/pkg/package.json",
        r#"{"main":"b.js"}"#,
    );
    assert_eq!(since(r), [VEND]);
    write(
        r,
        "packages/app/vendor/pkg/package.json",
        r#"{"main":"a.js","description":"x"}"#,
    );
    assert!(since(r).is_empty());
}

#[test]
fn imports_select_the_packages_own_files() {
    let d = repo();
    let r = d.path();
    edit(
        r,
        r##""#cfg": "./src/cfg.js""##,
        r##""#cfg": "./src/cfg2.js""##,
    );
    assert_eq!(since(r), [OWN_CFG]);
    edit(
        r,
        r##""#gen/*": "./src/gen/*.js""##,
        r##""#gen/*": "./src/gen2/*.js""##,
    );
    assert_eq!(since(r), [OWN_GEN]);
    // An added specifier nobody uses.
    edit(
        r,
        r##""#cfg": "./src/cfg.js""##,
        r##""#cfg": "./src/cfg.js", "#new": "./src/alt.js""##,
    );
    assert!(since(r).is_empty());
}

#[test]
fn type_entries_reach_only_entries_that_type_check() {
    let d = repo();
    let r = d.path();
    let of = |entries: &[&str]| -> Vec<String> {
        let mut args = vec!["affected", "--since", "HEAD", "--on-uncertain", "ignore"];
        args.extend(entries);
        ok_env(r, &args, &[]).lines().map(str::to_owned).collect()
    };
    // What type resolution finds for `./util` changes. The importing package's
    // tests run under jest, which transpiles file by file: they cannot observe
    // it. A plain entry (type-checked as far as the trace knows) is selected.
    edit(
        r,
        r#""types": "./src/util.d.ts", "default""#,
        r#""types": "./src/util2.d.ts", "default""#,
    );
    assert!(since(r).is_empty());
    assert_eq!(of(&[UTIL_SRC, ROOT_SRC, FEAT_SRC]), [UTIL_SRC]);
    let err = stats(r);
    assert!(
        err.contains("0 importer(s) (+1 for type-checking entries only)"),
        "{err}"
    );
    // `typesVersions`: the subpath it maps, and the root request (its types entry
    // is mapped through all of it); not other subpaths.
    edit(
        r,
        r#""util": ["src/util.d.ts"]"#,
        r#""util": ["src/util2.d.ts"]"#,
    );
    assert!(since(r).is_empty());
    assert_eq!(of(&[UTIL_SRC, ROOT_SRC, FEAT_SRC]), [ROOT_SRC, UTIL_SRC]);
    // The root's `types`: the importer of the root.
    edit(
        r,
        r#""types": "./src/index.d.ts""#,
        r#""types": "./src/alt.d.ts""#,
    );
    assert!(since(r).is_empty());
    assert_eq!(of(&[UTIL_SRC, ROOT_SRC, FEAT_SRC]), [ROOT_SRC]);
    // A runtime entry selects the jest test.
    edit(
        r,
        r#""default": "./src/util.js""#,
        r#""default": "./src/util2.js""#,
    );
    assert_eq!(since(r), [UTIL]);
}

#[test]
fn shapes_outside_the_model_are_whole_file_changes() {
    let d = repo();
    let r = d.path();
    let no_exports = LIB.replace(
        r#"  "exports": {
    ".": { "require": "./src/index.js", "default": "./src/index.js" },
    "./util": { "types": "./src/util.d.ts", "default": "./src/util.js" },
    "./feature/*": "./src/feature/*.js",
    "./internal/*": null
  },
"#,
        "",
    );
    assert_ne!(no_exports, LIB);
    // `exports` removed: every deep import changes meaning. The whole file counts,
    // and so do the requests that no longer resolve.
    write(r, "packages/lib/package.json", &no_exports);
    assert_eq!(since(r), [EXTRA, FEAT, ROOT, UTIL, OWN_CFG, OWN_GEN, PLAIN]);
    let err = stats(r);
    assert!(
        err.contains(
            "config packages/lib/package.json (exports): whole-file change: exports changed: \
             `exports` exists at one revision only"
        ),
        "{err}"
    );
    for (from, to) in [
        // A `browser` object remaps files.
        (
            r#""version": "1.0.0""#,
            r#""version": "1.0.0", "browser": {"./src/util.js": false}"#,
        ),
        // Subpaths and conditions mixed.
        (
            r#""./internal/*": null"#,
            r#""./internal/*": null, "import": "./src/alt.js""#,
        ),
        // Fields that are not resolution entries.
        (
            r#""version": "1.0.0""#,
            r#""version": "1.0.0", "type": "commonjs""#,
        ),
        (r#""name": "@acme/lib""#, r#""name": "@acme/lib2""#),
        (
            r#""version": "1.0.0""#,
            r#""version": "1.0.0", "somethingNew": true"#,
        ),
    ] {
        edit(r, from, to);
        let got = since(r);
        for t in [ROOT, UTIL, FEAT, OWN_CFG, PLAIN] {
            assert!(got.iter().any(|g| g == t), "{t} in {got:?} for {to}");
        }
    }
}
