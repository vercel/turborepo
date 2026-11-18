//! `affected` end to end for anchored names: a file that uses its
//! own location or the working directory names where that base goes
//! (`turborepo_trace_lang_js::flow::anchors`), not everything below it.
// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{path::Path, process::Command};

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

/// The tests a change of `changed` selects, with the graph in `graph` (one
/// directory per build mode: the anchors are followed when a file is traced).
fn affected(root: &Path, graph: &str, changed: &str, env: &[(&str, &str)]) -> Vec<String> {
    let mut c = Command::new(support::harness());
    c.args([
        "affected",
        "--tests",
        "jest",
        "--graph",
        graph,
        "--changed",
        changed,
    ])
    .current_dir(root);
    for (k, v) in env {
        c.env(k, v);
    }
    let o = c.output().expect("run the harness");
    assert!(
        o.status.success(),
        "turbotrace affected failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Why `entry` is selected by a change of `changed` (for assertion messages).
fn explain(root: &Path, graph: &str, changed: &str, entry: &str) -> String {
    let o = Command::new(support::harness())
        .args([
            "affected",
            "--tests",
            "jest",
            "--graph",
            graph,
            "--changed",
            changed,
            "--explain",
            entry,
        ])
        .current_dir(root)
        .output()
        .expect("run the harness");
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

/// Three services whose tests reach a reader with a parameter path (a named
/// read), and each a file that uses its location:
///
/// ```text
/// packages/reader/index.js     readText(p): fs.readFileSync(p)             p is a parameter
/// packages/envlib/index.ts     getEnv(name, fallback): process.env[name], or the fallback
/// packages/config/env.js       [cwd, cwd/../.., location/../..] each as `path` of dotenv-flow's config
/// packages/secrets/open.ts     join(location, '..', '..', getEnv('KEYS_PATH', '/etc/keys')) -> readText
/// packages/roots/index.js      exports.ROOT = resolve(location, '..', '..')
/// packages/plain/open.js       join(location, '..', '..', pick()) -> readText   pick is no env helper
/// services/a/test/a.test.js    config + secrets
/// services/b/test/b.test.js    roots
/// services/c/test/c.test.js    plain
/// ```
///
/// `KEYS_PATH` is assigned nowhere: the path with the environment tail stays an
/// environment read, which the default policy reports and does not count.
fn repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(r, "docs/guide.md", "guide\n");
    write(r, ".env.test", "A=1\n");
    for p in [
        "reader", "envlib", "config", "secrets", "roots", "plain", "other", "known",
    ] {
        write(
            r,
            &format!("packages/{p}/package.json"),
            &format!(r#"{{"name":"{p}","version":"1.0.0"}}"#),
        );
    }
    write(
        r,
        "packages/reader/index.js",
        "const fs = require('fs');\nexports.readText = function (p) {\n  return \
         fs.readFileSync(p, 'utf8');\n};\n",
    );
    write(
        r,
        "packages/envlib/index.ts",
        "export function getEnv(name: string, fallback: string): string {\n  const value = \
         process.env[name];\n  if (!value) {\n    return fallback;\n  }\n  return value;\n}\n",
    );
    write(
        r,
        "packages/other/index.js",
        "exports.pick = function () {\n  return globalThis.where;\n};\n",
    );
    write(
        r,
        "packages/config/env.js",
        "const { resolve } = require('path');\nconst flow = require('dotenv-flow');\nconst paths \
         = [\n  process.cwd(),\n  resolve(process.cwd(), '..', '..'),\n  resolve(__dirname, '..', \
         '..'),\n];\nfor (const path of paths) {\n  flow.config({ path: path, default_node_env: \
         'development' });\n}\n",
    );
    write(
        r,
        "packages/secrets/open.ts",
        "import { join, isAbsolute } from 'path';\nimport { getEnv } from '../envlib';\nimport { \
         readText } from '../reader';\nconst keysPath = getEnv('KEYS_PATH', '/etc/keys');\nexport \
         function open() {\n  const abs = isAbsolute(keysPath) ? keysPath : join(__dirname, '..', \
         '..', keysPath);\n  return readText(abs);\n}\n",
    );
    write(r, "etc/keys/k1", "key\n");
    // A repository of some size: the environment registry does not take "any
    // directory below the root" for a variable that is set outside the
    // repository as a list of files (more than 512 match), and leaves an
    // environment read.
    for i in 0..520 {
        write(r, &format!("data/bulk/f{i}.txt"), "x\n");
    }
    write(
        r,
        "packages/roots/index.js",
        "const { resolve } = require('path');\nconst { readText } = \
         require('../reader');\nexports.ROOT = resolve(__dirname, '..', '..');\nexports.read = \
         readText;\n",
    );
    write(
        r,
        "packages/plain/open.js",
        "const { join } = require('path');\nconst { pick } = require('../other');\nconst { \
         readText } = require('../reader');\nexports.open = function () {\n  return \
         readText(join(__dirname, '..', '..', pick()));\n};\n",
    );
    // A file path that no string spells, handed to the reader.
    write(
        r,
        "packages/known/index.js",
        "const path = require('path');\nconst { readText } = require('../reader');\nconst B = \
         __dirname;\nexports.known = function () {\n  for (const r of ['st' + 'uff' + '/' + 'da' \
         + 'ta']) {\n    readText(B === null ? r : path.join(B, '..', '..', r));\n  }\n};\n",
    );
    write(r, "stuff/data", "d\n");
    write(r, "stuff/other", "o\n");
    write(
        r,
        "services/d/test/d.test.js",
        "const { known } = require('../../../packages/known');\ntest('d', () => { known(); });\n",
    );
    for s in ["a", "b", "c", "d"] {
        write(
            r,
            &format!("services/{s}/package.json"),
            &format!(r#"{{"name":"svc-{s}","version":"1.0.0"}}"#),
        );
        write(
            r,
            &format!("services/{s}/jest.config.js"),
            "module.exports = {};\n",
        );
    }
    write(
        r,
        "services/a/test/a.test.js",
        "require('../../../packages/config/env');\nconst { open } = \
         require('../../../packages/secrets/open');\ntest('a', () => { open(); });\n",
    );
    write(r, "services/a/.env.local", "B=2\n");
    write(r, "services/a/notes.txt", "n\n");
    write(r, "services/.env", "C=3\n");
    write(
        r,
        "services/b/test/b.test.js",
        "const { ROOT, read } = require('../../../packages/roots');\ntest('b', () => { read(ROOT \
         + '/x'); });\n",
    );
    write(
        r,
        "services/c/test/c.test.js",
        "const { open } = require('../../../packages/plain/open');\ntest('c', () => { open(); \
         });\n",
    );
    d
}

const A: &str = "services/a/test/a.test.js";
const B: &str = "services/b/test/b.test.js";
const C: &str = "services/c/test/c.test.js";
const D: &str = "services/d/test/d.test.js";

#[test]
fn an_escaping_file_path_names_the_file() {
    let d = repo();
    let r = d.path();
    // Without following, the file names everything below the repository root (two
    // parent steps from its directory).
    assert!(
        affected(
            r,
            ".tt-coarse",
            "stuff/other",
            &[("TURBO_TRACE_DEBUG_ANCHORS", "0")]
        )
        .contains(&D.to_owned())
    );
    // Followed: the path is known, and names that file only.
    assert!(affected(r, ".tt", "stuff/data", &[]).contains(&D.to_owned()));
    assert!(
        !affected(r, ".tt", "stuff/other", &[]).contains(&D.to_owned()),
        "{}",
        explain(r, ".tt", "stuff/other", D)
    );
}

#[test]
fn a_followed_location_names_where_it_goes() {
    let d = repo();
    let r = d.path();
    // Fidelity: without following, each of the four files names everything below
    // the repository root, and an unrelated file selects every test.
    assert_eq!(
        affected(
            r,
            ".tt-coarse",
            "docs/guide.md",
            &[("TURBO_TRACE_DEBUG_ANCHORS", "0")]
        ),
        [A, B, C, D]
    );
    // Followed: the directories handed to dotenv-flow name their `.env*` files, and
    // the path with the environment tail is the environment registry's. An
    // exported root still names everything below it, and so does a path whose
    // tail is the result of a function that is no environment helper.
    assert_eq!(
        affected(r, ".tt", "docs/guide.md", &[]),
        [B, C],
        "{}",
        explain(r, ".tt", "docs/guide.md", A)
    );
    assert_eq!(affected(r, ".tt", "services/a/notes.txt", &[]), [B, C]);
    // The default of the helper is a string of the file: a name like any other.
    assert_eq!(affected(r, ".tt", "etc/keys/k1", &[]), [A, B, C]);
}

#[test]
fn directories_handed_to_dotenv_flow_name_env_files() {
    let d = repo();
    let r = d.path();
    // The repository root is two directories above the file.
    assert!(affected(r, ".tt", ".env.test", &[]).contains(&A.to_owned()));
    // The working directory of the test.
    assert!(affected(r, ".tt", "services/a/.env.local", &[]).contains(&A.to_owned()));
    // A directory that is none of the three.
    assert!(
        !affected(r, ".tt", "services/.env", &[]).contains(&A.to_owned()),
        "{}",
        explain(r, ".tt", "services/.env", A)
    );
}

/// A root that a file takes from its own location and only measures against or
/// hands to a child process as its working directory is no read: an unrelated
/// change selects nothing. A root that goes into a read selects, and the
/// attribution dump has a group for it.
#[test]
fn a_root_that_is_no_read_names_nothing() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(r, "docs/guide.md", "x\n");
    write(
        r,
        "lib/reader.js",
        "exports.read = (p) => require('fs').readFileSync(p, 'utf8');\n",
    );
    write(
        r,
        "services/a/package.json",
        r#"{"name":"a","version":"1.0.0"}"#,
    );
    write(r, "services/a/jest.config.js", "module.exports = {};\n");
    write(r, "services/a/data/x.txt", "x\n");
    write(
        r,
        "services/a/tools.js",
        "const path = require('path');\nconst { execSync } = require('child_process');\nconst { \
         read } = require('../../lib/reader');\nconst root = path.resolve(__dirname, '..', \
         '..');\nexports.rel = (f) => path.relative(root, f);\nexports.status = () => \
         execSync('git status --porcelain', { cwd: root });\nexports.inside = (f) => { if \
         (!f.startsWith(root)) throw new Error(`not below ${root}`); };\nexports.own = (name) => \
         read(path.join(__dirname, 'data', name));\n",
    );
    write(
        r,
        "services/a/up.js",
        "const path = require('path');\nconst { read } = \
         require('../../lib/reader');\nexports.top = (name) => read(path.join(process.cwd(), \
         '..', '..', name));\n",
    );
    const TOOLS: &str = "services/a/test/a.test.js";
    const UP: &str = "services/a/test/b.test.js";
    write(r, TOOLS, "require('../tools');\ntest('a', () => {});\n");
    write(r, UP, "require('../up');\ntest('b', () => {});\n");
    // The root of `tools.js` names nothing; the path `up.js` builds on the working
    // directory leads to the repository root and is read.
    assert_eq!(
        affected(r, ".tt", "docs/guide.md", &[]),
        [UP],
        "{}",
        explain(r, ".tt", "docs/guide.md", TOOLS)
    );
    assert!(
        explain(r, ".tt", "docs/guide.md", UP)
            .contains("the working directory (the repository root)")
    );
    // What `tools.js` reads below its own directory still selects (and `up.js`
    // names everything below the root).
    assert_eq!(
        affected(r, ".tt", "services/a/data/x.txt", &[]),
        [TOOLS, UP]
    );
    // Without the rules the root escapes and names every path.
    assert_eq!(
        affected(
            r,
            ".tt-off",
            "docs/guide.md",
            &[(
                "TURBO_TRACE_DEBUG_ANCHORS_OFF",
                "relative,child,tests,message"
            )]
        ),
        [TOOLS, UP]
    );
}

/// [`affected`] with more arguments.
fn affected_with(
    root: &Path,
    graph: &str,
    changed: &str,
    more: &[&str],
    env: &[(&str, &str)],
) -> Vec<String> {
    let mut c = Command::new(support::harness());
    c.args([
        "affected",
        "--tests",
        "jest",
        "--graph",
        graph,
        "--changed",
        changed,
    ])
    .args(more)
    .current_dir(root);
    for (k, v) in env {
        c.env(k, v);
    }
    let o = c.output().expect("run the harness");
    assert!(
        o.status.success(),
        "turbotrace affected failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// A program that code starts is outside the model, for every caller: with or
/// without a working directory, with a known or an unknown command line, by
/// default and under `--strict`, and with the rule for the working directory of
/// a child process on or off (`TURBO_TRACE_DEBUG_ANCHORS_OFF=child`). The call
/// leaves no record, so a test whose only tie to a file is such a child is not
/// selected by a change of it in any of these modes. That is the boundary of
/// the model (README, "Limitations"), not an assumption that `--strict`
/// could drop: the rule only keeps a located directory that is a child's
/// working directory from counting as a path of a read this process makes,
/// which matters where the test also loads a read with a computed path, and
/// there `--strict` selects with the rule on too.
#[test]
fn a_child_process_is_outside_the_model_for_every_caller() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(r, "docs/guide.md", "x\n");
    // What the children run and read: a script of the repository that reads a file.
    write(
        r,
        "scripts/check.js",
        "require('fs').readFileSync(require('path').join(__dirname, '..', 'docs', 'guide.md'));\n",
    );
    write(
        r,
        "services/a/package.json",
        r#"{"name":"a","version":"1.0.0"}"#,
    );
    write(r, "services/a/jest.config.js", "module.exports = {};\n");
    write(r, "services/a/data/x.txt", "x\n");
    // No working directory: the child inherits the test's.
    write(
        r,
        "services/a/tools/plain.js",
        "const { execSync } = require('child_process');\nexports.run = () => execSync('node \
         ../../scripts/check.js');\n",
    );
    // A located root as the working directory, a known command line.
    write(
        r,
        "services/a/tools/rooted.js",
        "const path = require('path');\nconst { execSync, spawnSync } = \
         require('child_process');\nconst root = path.resolve(__dirname, '..', '..', \
         '..');\nexports.status = () => execSync('git status --porcelain', { cwd: root \
         });\nexports.lint = () => spawnSync('pnpm', ['run', 'lint'], { cwd: root });\n",
    );
    // The same root, a command line that is not known.
    write(
        r,
        "services/a/tools/any.js",
        "const path = require('path');\nconst { spawnSync } = require('child_process');\nconst \
         root = path.resolve(__dirname, '..', '..', '..');\nexports.run = (cmd, args) => \
         spawnSync(cmd, args, { cwd: root });\n",
    );
    // The rooted child in a file that hands a path below its package to a reader: a
    // read with a computed path (a parameter), bounded by names.
    write(
        r,
        "services/a/tools/reads.js",
        "const path = require('path');\nconst { execSync } = require('child_process');\nconst { \
         read } = require('./reader');\nconst root = path.resolve(__dirname, '..', '..', \
         '..');\nexports.status = () => execSync('git status --porcelain', { cwd: root \
         });\nexports.own = (name) => read(path.join(__dirname, '..', 'data', name));\n",
    );
    write(
        r,
        "services/a/tools/reader.js",
        "exports.read = (p) => require('fs').readFileSync(p, 'utf8');\n",
    );
    const READS: &str = "services/a/test/reads.test.js";
    for name in ["plain", "rooted", "any", "reads"] {
        write(
            r,
            &format!("services/a/test/{name}.test.js"),
            &format!("require('../tools/{name}');\ntest('{name}', () => {{}});\n"),
        );
    }
    let none: [&str; 0] = [];
    let off = [("TURBO_TRACE_DEBUG_ANCHORS_OFF", "child")];
    for changed in ["docs/guide.md", "scripts/check.js"] {
        // The rule on and off, each with a graph of its own.
        for (graph, env) in [(".tt", &[][..]), (".tt-off", &off[..])] {
            // By default no test is selected. With the rule off the root of `reads.js`
            // escapes and names every path for its read: that test, and no other.
            let default = affected_with(r, graph, changed, &[], env);
            if env.is_empty() {
                assert_eq!(default, none, "{changed} {graph}");
            } else {
                assert_eq!(default, [READS], "{changed} {graph}");
            }
            // `--strict` counts the read with a computed path as open, with the rule on and
            // off alike; the tests that only start children have nothing to count.
            assert_eq!(
                affected_with(r, graph, changed, &["--strict"], env),
                [READS],
                "{changed} {graph} --strict"
            );
        }
    }
    // What the test's own process reads still selects it.
    assert_eq!(
        affected_with(r, ".tt", "services/a/data/x.txt", &[], &[]),
        [READS]
    );
}
