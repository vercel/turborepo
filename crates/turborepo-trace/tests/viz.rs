//! The impact report end to end (`affected --html`, `--format summary`;
//! `crate::viz`): the HTML is one self-contained file with valid data, the text
//! report is stable, and the selection is the same with and without the report.
// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::{Command, Output},
};

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

fn run(root: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut c = Command::new(support::harness());
    c.args(args).current_dir(root).env("COLUMNS", "100");
    c.env_remove("TURBO_TRACE_DEBUG_ATTR_DUMP");
    for (k, v) in env {
        c.env(k, v);
    }
    let o = c.output().expect("run turbotrace");
    assert!(
        o.status.success(),
        "turbotrace {args:?} failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    o
}

fn stdout(o: &Output) -> String {
    String::from_utf8(o.stdout.clone()).unwrap()
}

/// A small shop monorepo, committed, then changed in the working tree:
///
/// ```text
/// packages/ui        index.js re-exports button, card and modal; all three use theme.js
/// packages/format    money.js, date.js
/// packages/config    load.js requires a plugin named by an environment variable (open)
/// apps/shop          cart, catalog and checkout use the ui barrel; checkout loads config;
///                    search uses nothing shared; test/setup.js is a jest setup file
/// apps/admin         users uses the ui barrel; audit loads config; roles uses format/date
/// apps/docs          three tests of their own
/// ```
///
/// Changed: `packages/ui/src/theme.js` (through the barrel into three
/// packages), `packages/format/src/money.js`, `apps/shop/test/setup.js` (every
/// shop test), `README.md` (nothing depends on it) and `.github/CODEOWNERS` (a
/// steering file).
fn repo() -> Repo {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("acme");
    let r = root.as_path();
    write(
        r,
        "package.json",
        r#"{"name":"acme","private":true,"workspaces":["packages/*","apps/*"]}"#,
    );
    write(r, "README.md", "# acme\n");
    write(r, ".github/CODEOWNERS", "* @acme/web\n");
    let pkg = |dir: &str, name: &str, config: &str| {
        write(
            r,
            &format!("{dir}/package.json"),
            &format!(r#"{{"name":"{name}","version":"1.0.0"}}"#),
        );
        write(r, &format!("{dir}/jest.config.js"), config);
    };
    let plain = "module.exports = {};\n";
    pkg("packages/ui", "@acme/ui", plain);
    pkg("packages/format", "@acme/format", plain);
    pkg("packages/config", "@acme/config", plain);
    pkg(
        "apps/shop",
        "shop",
        "module.exports = { setupFiles: ['<rootDir>/test/setup.js'] };\n",
    );
    pkg("apps/admin", "admin", plain);
    pkg("apps/docs", "docs", plain);

    // packages/ui
    write(r, "packages/ui/src/theme.js", "exports.color = 'black';\n");
    for c in ["button", "card", "modal"] {
        write(
            r,
            &format!("packages/ui/src/{c}.js"),
            &format!("const theme = require('./theme');\nexports.{c} = () => theme.color;\n"),
        );
        write(
            r,
            &format!("packages/ui/test/{c}.test.js"),
            &format!("const {{ {c} }} = require('../src/{c}');\ntest('{c}', () => {c}());\n"),
        );
    }
    write(
        r,
        "packages/ui/src/index.js",
        "module.exports = { ...require('./button'), ...require('./card'), ...require('./modal') \
         };\n",
    );
    write(
        r,
        "packages/ui/test/index.test.js",
        "const ui = require('../src');\ntest('ui', () => ui.button());\n",
    );
    // packages/format
    for (f, body) in [
        ("money", "exports.money = (n) => '$' + n;\n"),
        ("date", "exports.date = (d) => String(d);\n"),
    ] {
        write(r, &format!("packages/format/src/{f}.js"), body);
        write(
            r,
            &format!("packages/format/test/{f}.test.js"),
            &format!("const {{ {f} }} = require('../src/{f}');\ntest('{f}', () => {f}(1));\n"),
        );
    }
    // packages/config
    write(
        r,
        "packages/config/src/load.js",
        "exports.load = function () {\n  return require(process.env.ACME_PLUGIN);\n};\n",
    );
    write(
        r,
        "packages/config/test/load.test.js",
        "const { load } = require('../src/load');\ntest('load', () => load);\n",
    );
    // apps/shop
    write(r, "apps/shop/test/setup.js", "global.shop = true;\n");
    let ui = "../../../packages/ui/src";
    let app = |app: &str, name: &str, body: &str, tests: usize| {
        write(r, &format!("apps/{app}/src/{name}.js"), body);
        for i in 1..=tests {
            write(
                r,
                &format!("apps/{app}/test/{name}-{i}.test.js"),
                &format!("const m = require('../src/{name}');\ntest('{name} {i}', () => m);\n"),
            );
        }
    };
    app(
        "shop",
        "cart",
        &format!(
            "const ui = require('{ui}');\nconst {{ money }} = \
             require('../../../packages/format/src/money');\nexports.cart = () => ui.button() + \
             money(1);\n"
        ),
        4,
    );
    app(
        "shop",
        "catalog",
        &format!("const ui = require('{ui}');\nexports.catalog = () => ui.card();\n"),
        3,
    );
    app(
        "shop",
        "checkout",
        &format!(
            "const ui = require('{ui}');\nconst {{ load }} = \
             require('../../../packages/config/src/load');\nexports.checkout = () => ui.modal() + \
             load();\n"
        ),
        3,
    );
    app("shop", "search", "exports.search = () => [];\n", 2);
    // apps/admin
    app(
        "admin",
        "users",
        &format!("const ui = require('{ui}');\nexports.users = () => ui.card();\n"),
        3,
    );
    app(
        "admin",
        "audit",
        "const { load } = require('../../../packages/config/src/load');\nexports.audit = () => \
         load();\n",
        2,
    );
    app(
        "admin",
        "roles",
        "const { date } = require('../../../packages/format/src/date');\nexports.roles = () => \
         date(0);\n",
        2,
    );
    // apps/docs
    app("docs", "pages", "exports.pages = [];\n", 3);

    git(r, &["init", "-q"]);
    git(r, &["add", "-A"]);
    git(r, &["commit", "-q", "-m", "base"]);

    write(r, "packages/ui/src/theme.js", "exports.color = 'white';\n");
    write(
        r,
        "packages/format/src/money.js",
        "exports.money = (n) => '€' + n;\n",
    );
    write(r, "apps/shop/test/setup.js", "global.shop = 1;\n");
    write(r, "README.md", "# acme shop\n");
    write(r, ".github/CODEOWNERS", "* @acme/shop\n");
    Repo { _dir: d, root }
}

/// The fixture: a directory named `acme` in a temporary directory.
struct Repo {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Repo {
    fn path(&self) -> &Path {
        &self.root
    }
}

const BASE: [&str; 4] = ["affected", "--tests", "jest", "--since=HEAD"];

fn args<'a>(extra: &[&'a str]) -> Vec<&'a str> {
    let mut a: Vec<&str> = BASE.to_vec();
    a.extend_from_slice(extra);
    a
}

/// The JSON embedded in the report.
fn data_of(html: &str) -> serde_json::Value {
    let start = html
        .find(r#"<script type="application/json" id="data">"#)
        .expect("the data element");
    let body = &html[start..];
    let body = &body[body.find('>').unwrap() + 1..];
    let end = body.find("</script>").expect("the end of the data element");
    serde_json::from_str(&body[..end]).expect("the embedded data is valid JSON")
}

#[test]
fn html_is_one_self_contained_file() {
    let d = repo();
    let r = d.path();
    let list = stdout(&run(r, &args(&[]), &[]));
    let out = r.join("out/report.html");
    let o = run(r, &args(&[&format!("--html={}", out.display())]), &[]);
    // The list is the same with the report; the path is on stderr.
    assert_eq!(stdout(&o), list);
    let err = String::from_utf8_lossy(&o.stderr).into_owned();
    assert!(err.contains("impact report: "), "{err}");
    assert!(err.contains("report.html"), "{err}");
    let html = std::fs::read_to_string(&out).unwrap();
    for needle in [
        "http://",
        "https://",
        "<script src",
        "<link",
        "@import",
        "<img",
        "<iframe",
        "fetch(",
        "XMLHttpRequest",
    ] {
        assert!(
            !html.contains(needle),
            "the report must not contain {needle}"
        );
    }
    assert!(
        html.match_indices("url(")
            .all(|(i, _)| html[i..].starts_with("url(#")),
        "the only url() is a reference inside the page"
    );
    assert!(html.starts_with("<!doctype html>"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&out).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "the report holds private paths");
    }

    let v = data_of(&html);
    let selected: Vec<&str> = list.lines().collect();
    assert_eq!(v["totals"]["selected"], selected.len());
    assert_eq!(v["totals"]["entries"], 29);
    // Every selected entry is in the data with a cause, a source and its chain.
    let nodes = v["nodes"].as_array().unwrap();
    let mut seen = Vec::new();
    for e in v["entries"].as_array().unwrap() {
        let path = nodes[e[0].as_u64().unwrap() as usize].as_str().unwrap();
        if e[3] == 1 {
            seen.push(path);
            assert!(e[4].as_i64().unwrap() >= 0, "{path} has a cause");
            assert!(e[6].as_i64().unwrap() >= 0, "{path} has a source");
            assert!(!e[9].as_array().unwrap().is_empty(), "{path} has a chain");
        }
    }
    seen.sort_unstable();
    assert_eq!(seen, selected);
    // The barrel holds the tests that reach the theme only through it.
    let hubs = v["hubs"].as_array().unwrap();
    let barrel = hubs
        .iter()
        .find(|h| nodes[h["node"].as_u64().unwrap() as usize] == "packages/ui/src/index.js")
        .expect("the barrel is a hub");
    assert_eq!(barrel["mech"], "change");
    assert!(barrel["own"].as_u64().unwrap() >= 3, "{barrel}");
    // The changed files and what became of them.
    let status: BTreeMap<&str, &str> = v["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["path"].as_str().unwrap(), c["status"].as_str().unwrap()))
        .collect();
    assert_eq!(status["packages/ui/src/theme.js"], "seed");
    assert_eq!(status[".github/CODEOWNERS"], "steering");
    assert_eq!(status["README.md"], "unmatched");
    // No file contents: the changed line of the theme is not in the report.
    assert!(!html.contains("exports.color"));

    // The default file is in the graph directory.
    let o = run(r, &args(&["--html"]), &[]);
    assert_eq!(stdout(&o), list);
    assert!(r.join(".turbo/trace/impact.html").is_file());
    // --open does nothing without a terminal.
    let o = run(r, &args(&["--open"]), &[("CI", "1")]);
    assert!(String::from_utf8_lossy(&o.stderr).contains("--open ignored"));
}

fn golden(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/viz")
        .join(name)
}

#[test]
fn summary_is_stable() {
    let d = repo();
    let r = d.path();
    let text = stdout(&run(r, &args(&["--format", "summary"]), &[]));
    assert!(!text.contains('\x1b'), "no colour without a terminal");
    assert!(text.lines().all(|l| l.chars().count() <= 100), "{text}");
    let path = golden("summary.txt");
    if std::env::var_os("TURBO_TRACE_DEBUG_UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &text).unwrap();
    }
    let want = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(
        text, want,
        "the summary changed; rerun with TURBO_TRACE_DEBUG_UPDATE_GOLDEN=1 to accept it"
    );
    // A narrow terminal still fits.
    let narrow = stdout(&run(
        r,
        &args(&["--format", "summary"]),
        &[("COLUMNS", "60")],
    ));
    assert!(narrow.lines().all(|l| l.chars().count() <= 60), "{narrow}");
}

/// A manual mock that a helper registers is a dependency of the mocked module,
/// for the tests that load the helper (`affected_mocks.rs`). A test that gets
/// the mock is drawn with the chain over it, as an import of the changed file.
/// A test that requests the module without the helper and reaches the change in
/// another, longer way is drawn with that way: the shortest chain of the view
/// would cross a mock the test does not get.
#[test]
fn a_chain_crosses_a_manual_mock_only_for_the_tests_that_get_it() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(r, ".gitignore", ".turbo/trace/\n");
    write(
        r,
        "jest.config.js",
        "module.exports = { testMatch: ['**/test/**/*.test.ts'] };\n",
    );
    write(
        r,
        "src/helper.ts",
        "jest.mock('./n');\nexport const h = 1;\n",
    );
    write(r, "src/n.ts", "export const n = 1;\n");
    write(
        r,
        "src/__mocks__/n.ts",
        "import { md } from '../mockdep';\nexport const n = md;\n",
    );
    write(r, "src/mockdep.ts", "export const md = 2;\n");
    write(
        r,
        "src/x2.ts",
        "import { md } from './mockdep';\nexport const x2 = md;\n",
    );
    write(
        r,
        "src/x1.ts",
        "import { x2 } from './x2';\nexport const x1 = x2;\n",
    );
    write(
        r,
        "src/x0.ts",
        "import { x1 } from './x1';\nexport const x0 = x1;\n",
    );
    // The helper and the module: the mock.
    write(
        r,
        "test/b.test.ts",
        "import '../src/helper';\nimport { n } from '../src/n';\ntest('b', () => { n; });\n",
    );
    // The module without the helper: the real module.
    write(
        r,
        "test/c.test.ts",
        "import { n } from '../src/n';\ntest('c', () => { n; });\n",
    );
    // The same, and another way to the change.
    write(
        r,
        "test/f.test.ts",
        "import { n } from '../src/n';\nimport { x0 } from '../src/x0';\ntest('f', () => { n; x0; \
         });\n",
    );
    let out = r.join("out/report.html");
    let html_arg = format!("--html={}", out.display());
    let base = ["affected", "--tests", "jest", "--changed", "src/mockdep.ts"];
    let list = stdout(&run(r, &[&base[..], &[html_arg.as_str()]].concat(), &[]));
    assert_eq!(
        list.lines().collect::<Vec<_>>(),
        ["test/b.test.ts", "test/f.test.ts"]
    );
    let data = data_of(&std::fs::read_to_string(&out).unwrap());
    let name = |i: &serde_json::Value| data["nodes"][i.as_u64().unwrap() as usize].clone();
    let row = |path: &str| -> serde_json::Value {
        data["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| name(&e[0]) == path)
            .unwrap_or_else(|| panic!("no row for {path}"))
            .clone()
    };
    let chain = |path: &str| -> Vec<String> {
        row(path)[9]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| name(i).as_str().unwrap().to_owned())
            .collect()
    };
    let cause = |path: &str| -> serde_json::Value {
        data["causes"][row(path)[4].as_u64().unwrap() as usize]["id"].clone()
    };
    assert_eq!(
        chain("test/b.test.ts"),
        [
            "src/mockdep.ts",
            "src/__mocks__/n.ts",
            "src/n.ts",
            "test/b.test.ts"
        ]
    );
    assert_eq!(
        chain("test/f.test.ts"),
        [
            "src/mockdep.ts",
            "src/x2.ts",
            "src/x1.ts",
            "src/x0.ts",
            "test/f.test.ts"
        ]
    );
    for t in ["test/b.test.ts", "test/f.test.ts"] {
        assert_eq!(cause(t), "import", "{t}");
        assert_eq!(row(t)[3], 1, "{t}: selected");
        // The chain is the one `--explain` prints: no second chain.
        assert_eq!(row(t)[15], serde_json::json!([]), "{t}");
    }
    assert_eq!(row("test/c.test.ts")[3], 0, "not selected");
    // The text report counts the same entries.
    let text = stdout(&run(
        r,
        &[&base[..], &["--format", "summary"]].concat(),
        &[],
    ));
    assert!(text.contains("2 of 3 entries selected"), "{text}");
    assert!(text.contains("Imports a changed file  2"), "{text}");
}

/// Keeps the report of the fixture for the documentation:
/// `TURBO_TRACE_DEBUG_VIZ_KEEP=<dir> cargo test -p turborepo-trace --test viz
/// keep`.
#[test]
fn keep_the_demo_report() {
    let Some(dir) = std::env::var_os("TURBO_TRACE_DEBUG_VIZ_KEEP") else {
        return;
    };
    let dir = PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    let d = repo();
    let r = d.path();
    let out = dir.join("acme-impact.html");
    run(r, &args(&[&format!("--html={}", out.display())]), &[]);
    let text = stdout(&run(r, &args(&["--format", "summary"]), &[]));
    std::fs::write(dir.join("acme-summary.txt"), text).unwrap();
}
