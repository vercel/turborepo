//! End-to-end tests of references next to a located module
//! ([`super::located`]): real sources, the real extractor and resolver, and
//! `node_modules` trees in which resolving from the wrong place gives another
//! file.

use std::{collections::BTreeSet, path::Path};

use turborepo_trace_core::{Externals, Tracer, UncertaintyKind};

use crate::JsLanguage;

/// Writes `files` (path, content) below `root`.
fn tree(root: &Path, files: &[(&str, &str)]) {
    for (p, text) in files {
        let path = root.join(p);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, text).expect("write");
    }
}

/// A project with three copies of `dep`: next to `pkg` (the referencing
/// package), next to `other` (a sibling directory) and inside `eve` (a
/// dependency).
fn project(root: &Path, eve: bool) {
    tree(
        root,
        &[
            ("package.json", r#"{"name":"root"}"#),
            (
                "node_modules/dep/package.json",
                r#"{"name":"dep","main":"index.js"}"#,
            ),
            ("node_modules/dep/index.js", "module.exports = 'root';"),
            ("pkg/package.json", r#"{"name":"pkg"}"#),
            (
                "pkg/node_modules/dep/package.json",
                r#"{"name":"dep","main":"index.js"}"#,
            ),
            ("pkg/node_modules/dep/index.js", "module.exports = 'pkg';"),
            ("pkg/src/b.cjs", "module.exports = 'b';"),
            ("other/package.json", r#"{"name":"other"}"#),
            ("other/x.js", "module.exports = 'x';"),
            (
                "other/node_modules/dep/package.json",
                r#"{"name":"dep","main":"index.js"}"#,
            ),
            (
                "other/node_modules/dep/index.js",
                "module.exports = 'other';",
            ),
        ],
    );
    if eve {
        tree(
            root,
            &[
                (
                    "node_modules/eve/package.json",
                    r#"{"name":"eve","exports":{".":{"import":"./dist/index.mjs","require":"./cjs/index.cjs"},"./package.json":"./package.json"}}"#,
                ),
                ("node_modules/eve/dist/index.mjs", "export default 1;"),
                ("node_modules/eve/dist/inner.js", "module.exports = 2;"),
                ("node_modules/eve/dist/deep/leaf.js", "module.exports = 3;"),
                ("node_modules/eve/cjs/index.cjs", "module.exports = 4;"),
                ("node_modules/eve/cjs/inner.js", "module.exports = 5;"),
                (
                    "node_modules/eve/node_modules/dep/package.json",
                    r#"{"name":"dep","main":"index.js"}"#,
                ),
                (
                    "node_modules/eve/node_modules/dep/index.js",
                    "module.exports = 'eve';",
                ),
            ],
        );
    }
}

struct Run {
    files: BTreeSet<String>,
    /// The third-party packages the trace stopped at.
    externals: Vec<String>,
    /// `(kind, spec of the edge)` of every uncertainty.
    open: Vec<(UncertaintyKind, String)>,
}

fn run(eve: bool, name: &str, source: &str) -> Run {
    run_with(eve, &[], name, source)
}

/// [`run`] in a project with the files `extra` added.
fn run_with(eve: bool, extra: &[(&str, &str)], name: &str, source: &str) -> Run {
    run_as(Externals::Follow, eve, extra, name, source)
}

/// [`run_with`] under an externals policy.
fn run_as(
    externals: Externals,
    eve: bool,
    extra: &[(&str, &str)],
    name: &str,
    source: &str,
) -> Run {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().canonicalize().expect("canonicalize");
    project(&root, eve);
    tree(&root, extra);
    let entry = root.join("pkg/src").join(name);
    std::fs::write(&entry, source).expect("write");
    let trace = Tracer::builder()
        .language(JsLanguage)
        .externals(externals)
        .base(&root)
        .cwd(&root)
        .build()
        .expect("tracer")
        .trace(&[entry])
        .expect("trace");
    let files = trace
        .files()
        .map(|p| {
            p.strip_prefix(&root)
                .unwrap_or(p)
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    let open = trace
        .uncertainties()
        .into_iter()
        .map(|(_, u, e)| (u.kind, e.map(|e| e.spec.to_string()).unwrap_or_default()))
        .collect();
    let externals = trace.externals().into_iter().map(str::to_owned).collect();
    Run {
        files,
        externals,
        open,
    }
}

impl Run {
    fn has(&self, f: &str) -> bool {
        self.files.contains(f)
    }

    #[track_caller]
    fn loads(&self, yes: &[&str], no: &[&str]) {
        for f in yes {
            assert!(
                self.has(f),
                "{f} missing from {:?} ({:?})",
                self.files,
                self.open
            );
        }
        for f in no {
            assert!(!self.has(f), "{f} in {:?}", self.files);
        }
    }
}

const ESM: &str = "import { createRequire } from 'node:module';\nimport { pathToFileURL, \
                   fileURLToPath } from 'node:url';\nimport path from 'node:path';\nconst require \
                   = createRequire(import.meta.url);\n";
const CJS: &str = "const { createRequire } = require('node:module');\nconst { pathToFileURL } = \
                   require('node:url');\nconst path = require('node:path');\n";

#[test]
fn import_of_a_url_next_to_import_meta_resolve() {
    // `import.meta.resolve` finds the `import` entry: the directory is `dist`.
    let r = run(
        true,
        "a.mjs",
        &format!(
            "{ESM}const root = new URL('.', import.meta.resolve('eve'));\nexport const m = await \
             import(new URL('inner.js', root).href);\nexport const n = await import(new \
             URL('./deep/leaf.js', import.meta.resolve('eve')));\n"
        ),
    );
    r.loads(
        &[
            "node_modules/eve/dist/index.mjs",
            "node_modules/eve/dist/inner.js",
            "node_modules/eve/dist/deep/leaf.js",
        ],
        &["node_modules/eve/cjs/inner.js"],
    );
    assert_eq!(r.open, vec![]);
}

#[test]
fn import_of_a_path_next_to_require_resolve() {
    // `require.resolve` finds the `require` entry: the directory is `cjs`. A
    // package root comes from its `package.json`.
    let r = run(
        true,
        "a.mjs",
        &format!(
            "{ESM}const entry = \
             pathToFileURL(createRequire(import.meta.url).resolve('eve')).href;\nexport const m = \
             await import(new URL('./inner.js', entry).href);\nconst root = \
             path.dirname(require.resolve('eve/package.json'));\nexport const n = await \
             import(pathToFileURL(path.join(root, 'dist/deep/leaf.js')).href);\nexport const o = \
             await import(`${{root}}/dist/index.mjs`);\n"
        ),
    );
    r.loads(
        &[
            "node_modules/eve/cjs/index.cjs",
            "node_modules/eve/cjs/inner.js",
            "node_modules/eve/dist/deep/leaf.js",
            "node_modules/eve/dist/index.mjs",
        ],
        &["node_modules/eve/dist/inner.js"],
    );
    assert_eq!(r.open, vec![]);
}

#[test]
fn import_of_exactly_the_resolved_file() {
    // `import(pathToFileURL(require.resolve(m)))` loads the `require` entry, not
    // what `import(m)` would.
    let r = run(
        true,
        "a.mjs",
        &format!("{ESM}export const m = await import(pathToFileURL(require.resolve('eve')));\n"),
    );
    r.loads(
        &["node_modules/eve/cjs/index.cjs"],
        &["node_modules/eve/dist/index.mjs"],
    );
    assert_eq!(r.open, vec![]);
}

#[test]
fn several_values_are_several_files() {
    let r = run(
        true,
        "a.mjs",
        &format!(
            "{ESM}const root = new URL('.', import.meta.resolve('eve'));\nconst load = (p) => \
             import(new URL(p, root).href);\nawait load('inner.js'); await load('deep/leaf.js');\n"
        ),
    );
    r.loads(
        &[
            "node_modules/eve/dist/inner.js",
            "node_modules/eve/dist/deep/leaf.js",
        ],
        &[],
    );
    assert_eq!(r.open, vec![]);
}

#[test]
fn a_computed_rest_is_a_glob_below_the_module() {
    let r = run(
        true,
        "a.mjs",
        &format!(
            "{ESM}const root = new URL('dist/', \
             pathToFileURL(require.resolve('eve/package.json')));\nexport const load = (p) => \
             import(new URL(p, root).href);\n"
        ),
    );
    r.loads(
        &[
            "node_modules/eve/dist/index.mjs",
            "node_modules/eve/dist/inner.js",
            "node_modules/eve/dist/deep/leaf.js",
        ],
        &["node_modules/eve/cjs/inner.js"],
    );
}

#[test]
fn require_resolve_with_paths_searches_from_the_entry() {
    let r = run(
        true,
        "c.cjs",
        &format!(
            "{CJS}const a = require(require.resolve('dep', {{ paths: \
             [require.resolve('eve/package.json')] }}));\nconst b = \
             require(require.resolve('dep/package.json', {{ paths: [path.join(__dirname, \
             '../../other')] }}));\n"
        ),
    );
    r.loads(
        &[
            "node_modules/eve/node_modules/dep/index.js",
            "other/node_modules/dep/package.json",
        ],
        &[
            "pkg/node_modules/dep/index.js",
            "pkg/node_modules/dep/package.json",
            "node_modules/dep/index.js",
        ],
    );
    assert_eq!(r.open, vec![]);
}

#[test]
fn require_resolve_with_paths_joins_a_relative_request() {
    let r = run(
        true,
        "c.cjs",
        &format!(
            "{CJS}const a = require(require.resolve('./x.js', {{ paths: [path.join(__dirname, \
             '../../other')] }}));\nconst b = require(require.resolve('./inner', {{ paths: \
             [path.dirname(require.resolve('eve'))] }}));\n"
        ),
    );
    r.loads(&["other/x.js", "node_modules/eve/cjs/inner.js"], &[]);
    assert_eq!(r.open, vec![]);
}

#[test]
fn options_that_are_not_understood_stay_open() {
    // An unknown entry or options from elsewhere: no reference from the file.
    for call in [
        "require.resolve('dep', { paths: [dir] })",
        "require.resolve('dep', opts)",
        "require.resolve('dep', { paths })",
    ] {
        let r = run(
            true,
            "c.cjs",
            &format!("{CJS}module.exports = (dir, opts, paths) => require({call});\n"),
        );
        r.loads(&[], &["pkg/node_modules/dep/index.js"]);
        assert!(
            r.open
                .iter()
                .any(|(k, _)| *k == UncertaintyKind::ComputedSpecifier),
            "{call}: {:?}",
            r.open
        );
    }
    // A spread may or may not bring `paths`: from the file, or unknown.
    let r = run(
        true,
        "c.cjs",
        &format!(
            "{CJS}module.exports = (opts) => require(require.resolve('dep', {{ ...opts }}));\n"
        ),
    );
    r.loads(&["pkg/node_modules/dep/index.js"], &[]);
    assert!(
        r.open
            .iter()
            .any(|(k, _)| *k == UncertaintyKind::ComputedSpecifier),
        "{:?}",
        r.open
    );
    // No options, or none that matter: from the file.
    for call in [
        "require.resolve('dep', undefined)",
        "require.resolve('dep', {})",
        "require.resolve('dep', { paths: undefined })",
    ] {
        let r = run(true, "c.cjs", &format!("{CJS}const a = require({call});\n"));
        r.loads(&["pkg/node_modules/dep/index.js"], &[]);
        assert_eq!(r.open, vec![], "{call}");
    }
}

#[test]
fn create_require_searches_from_its_anchor() {
    let r = run(
        true,
        "c.cjs",
        &format!(
            "{CJS}const other = createRequire(path.join(__dirname, \
             '../../other/package.json'));\nconst a = other('./x.js');\nconst b = \
             other('dep');\nconst c = other.resolve('dep/package.json');\n"
        ),
    );
    r.loads(
        &[
            "other/x.js",
            "other/node_modules/dep/index.js",
            "other/node_modules/dep/package.json",
        ],
        // Not from the file.
        &[
            "pkg/node_modules/dep/index.js",
            "pkg/node_modules/dep/package.json",
        ],
    );
    assert_eq!(r.open, vec![]);
}

#[test]
fn a_third_party_target_is_its_package_unless_externals_are_followed() {
    // A file next to a module, a module searched from a dependency, a glob below a
    // module: each is the package the file lies in, as for a bare import of it.
    let r = run_as(
        Externals::Stop,
        true,
        &[],
        "a.mjs",
        &format!(
            "{ESM}const root = new URL('.', import.meta.resolve('eve'));\nexport const m = await \
             import(new URL('inner.js', root).href);\nexport const n = \
             require(require.resolve('dep', {{ paths: [require.resolve('eve/package.json')] \
             }}));\nexport const load = (p) => import(new URL(p, root).href);\n"
        ),
    );
    assert!(
        r.files.iter().all(|f| !f.contains("node_modules")),
        "{:?}",
        r.files
    );
    for package in ["eve", "dep"] {
        assert!(
            r.externals.iter().any(|e| e == package),
            "{package}: {:?}",
            r.externals
        );
    }
}

#[test]
fn a_builtin_module_is_the_same_from_every_place() {
    let r = run(
        true,
        "c.cjs",
        &format!(
            "{CJS}const other = createRequire(path.join(__dirname, '../../other/a.js'));\nconst a \
             = other('fs');\nconst b = require(require.resolve('os', {{ paths: \
             [require.resolve('eve')] }}));\n"
        ),
    );
    assert_eq!(r.open, vec![]);
}

#[test]
fn create_require_inline() {
    let r = run(
        true,
        "c.cjs",
        &format!(
            "{CJS}const a = createRequire(path.join(__dirname, '..', \
             'package.json'))('./src/b.cjs');\nconst b = \
             createRequire(__filename).resolve('dep');\nconst c = \
             createRequire(require.resolve('eve/package.json'))('dep');\nconst d = \
             createRequire(__dirname + '/')('./b.cjs');\n"
        ),
    );
    r.loads(
        &[
            "pkg/src/b.cjs",
            "pkg/node_modules/dep/index.js",
            "node_modules/eve/node_modules/dep/index.js",
        ],
        &["node_modules/dep/index.js"],
    );
    assert_eq!(r.open, vec![]);
}

#[test]
fn create_require_with_an_unknown_anchor_stays_open() {
    for decl in [
        "module.exports = (dir) => createRequire(dir)('dep');",
        "module.exports = (dir) => { const r = createRequire(dir); return r('dep'); };",
        "module.exports = (dir) => { const require = createRequire(dir); return require('dep'); };",
        "let r = createRequire(__filename); r = createRequire(process.cwd() + '/'); \
         module.exports = r('dep');",
    ] {
        let r = run(true, "c.cjs", &format!("{CJS}{decl}\n"));
        assert!(
            r.open
                .iter()
                .any(|(k, _)| *k == UncertaintyKind::ComputedSpecifier),
            "{decl}: {:?}",
            r.open
        );
    }
    // A name declared again in an inner scope is its own `require`.
    let r = run(
        true,
        "c.cjs",
        &format!(
            "{CJS}const r = createRequire(path.join(__dirname, \
             '../../other/package.json'));\nfunction f() {{ const r = createRequire(__filename); \
             return r('dep'); }}\nmodule.exports = [f, r('./x.js')];\n"
        ),
    );
    r.loads(&["pkg/node_modules/dep/index.js", "other/x.js"], &[]);
    assert_eq!(r.open, vec![]);
}

/// One record: the load of `eve/x` searched from `other/`, whose edge names the
/// package and the directory of the package whose lockfile entry decides.
fn searched_from_other(open: &[(UncertaintyKind, String)]) -> bool {
    matches!(open, [(UncertaintyKind::UnresolvedSpecifier, s)]
        if s.starts_with("require.resolve('eve/x', from '../../other') (not installed: eve from /")
            && s.ends_with("/other)"))
}

#[test]
fn a_module_that_is_not_installed() {
    // The load is exactly the module: an unresolved edge with the bare specifier
    // (which a lockfile can link), nothing open.
    let r = run(
        false,
        "a.mjs",
        &format!("{ESM}export const m = await import(pathToFileURL(require.resolve('eve')));\n"),
    );
    assert!(
        r.open
            .iter()
            .all(|(k, _)| *k == UncertaintyKind::UnresolvedSpecifier),
        "{:?}",
        r.open
    );
    assert!(r.open.iter().any(|(_, s)| s == "eve"), "{:?}", r.open);
    // A path next to it, in a package that can only come from the registry: the
    // load is taken to stay in that package (not counted by default, open under
    // `--strict`).
    let next = format!(
        "{ESM}export const m = await import(new URL('./inner.js', \
         import.meta.resolve('eve')).href);\n"
    );
    let r = run(false, "a.mjs", &next);
    assert!(
        r.open
            .iter()
            .any(|(k, s)| *k == UncertaintyKind::UnresolvedSpecifier
                && s == "import.meta.resolve('eve')/../inner.js (not installed: eve)"),
        "{:?}",
        r.open
    );
    assert!(
        r.open
            .iter()
            .all(|(k, _)| *k == UncertaintyKind::UnresolvedSpecifier),
        "{:?}",
        r.open
    );
    assert!(r.open.iter().any(|(_, s)| s == "eve"), "{:?}", r.open);
    // Several values, one module: the reference of `import.meta.resolve`, one edge
    // for the module and one for the load; no record of the reference itself.
    let r = run(
        false,
        "a.mjs",
        &format!(
            "{ESM}const root = new URL('.', import.meta.resolve('eve'));\nconst load = (p) => \
             import(new URL(p, root).href);\nawait load('a.js'); await load('b.js');\n"
        ),
    );
    let both = "import.meta.resolve('eve')/../a.js | import.meta.resolve('eve')/../b.js";
    assert_eq!(
        r.open,
        vec![
            (UncertaintyKind::UnresolvedSpecifier, "eve".to_owned()),
            (UncertaintyKind::UnresolvedSpecifier, "eve".to_owned()),
            (
                UncertaintyKind::UnresolvedSpecifier,
                format!("{both} (not installed: eve)")
            ),
        ]
    );
    // The same for a module searched from a package that is not installed.
    let r = run(
        false,
        "c.cjs",
        &format!("{CJS}const m = createRequire(require.resolve('eve'))('dep');\n"),
    );
    assert!(
        r.open
            .iter()
            .all(|(k, _)| *k == UncertaintyKind::UnresolvedSpecifier),
        "{:?}",
        r.open
    );
    r.loads(&[], &["pkg/node_modules/dep/index.js"]);
    // A package of that name in the repository changes nothing here: whether the
    // module is a package of the registry or of the repository is the
    // lockfile's answer for the importer, which the graph store gives (it makes
    // the record open for a workspace link). The edge says which package's
    // entry decides.
    let r = run_with(
        false,
        &[("packages/eve/package.json", r#"{"name":"eve"}"#)],
        "a.mjs",
        &next,
    );
    assert!(
        r.open
            .iter()
            .any(|(k, s)| *k == UncertaintyKind::UnresolvedSpecifier
                && s == "import.meta.resolve('eve')/../inner.js (not installed: eve)"),
        "{:?}",
        r.open
    );
    // A registry package searched from another package of the repository is decided
    // by that package's lockfile entry: the reference depends on that package's
    // manifest and on the lockfile (here a missing one: a probe).
    let r = run(
        false,
        "c.cjs",
        &format!(
            "{CJS}const m = createRequire(path.join(__dirname, '../../other/a.js'))('eve/x');\n"
        ),
    );
    assert!(searched_from_other(&r.open), "{:?}", r.open);
    r.loads(&["other/package.json"], &[]);
    // The same with a package of that name in the repository (the store decides).
    let r = run_with(
        false,
        &[("packages/eve/package.json", r#"{"name":"eve"}"#)],
        "c.cjs",
        &format!(
            "{CJS}const m = createRequire(path.join(__dirname, '../../other/a.js'))('eve/x');\n"
        ),
    );
    assert!(searched_from_other(&r.open), "{:?}", r.open);
    // With a lockfile the reference reads it.
    let r = run_with(
        false,
        &[("pnpm-lock.yaml", "lockfileVersion: '9.0'\n")],
        "c.cjs",
        &format!(
            "{CJS}const m = createRequire(path.join(__dirname, '../../other/a.js'))('eve/x');\n"
        ),
    );
    assert!(searched_from_other(&r.open), "{:?}", r.open);
    r.loads(&["pnpm-lock.yaml", "other/package.json"], &[]);
    // From the file's own package the lockfile entry is the answer: nothing open.
    let r = run(
        false,
        "c.cjs",
        &format!("{CJS}const m = createRequire(path.join(__dirname, '../a.js'))('eve');\n"),
    );
    assert_eq!(
        r.open,
        vec![(UncertaintyKind::UnresolvedSpecifier, "eve".to_owned())]
    );
}
