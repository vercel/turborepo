//! End-to-end graph builds on a generated pnpm-style workspace and on a
//! mixed-language repository (JS, Go, Rust, and Python files, which no language
//! traces here), and the composition property tests: after random edits,
//! the graph composed from an incrementally rebuilt store is byte-identical
//! (JSON export and shard keys) to a fresh full build of the same tree.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use proptest::prelude::*;
use turborepo_trace_core::Language;
use turborepo_trace_graph::{Engine, GraphView};
use turborepo_trace_store::{Externals, Graph, GraphOptions, build};

fn langs() -> Vec<Arc<dyn Language>> {
    vec![Arc::new(turborepo_trace_lang_js::JsLanguage)]
}

fn write(root: &Path, rel: &str, contents: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, contents).unwrap();
}

fn symlink(root: &Path, target: &str, link: &str) {
    let l = root.join(link);
    std::fs::create_dir_all(l.parent().unwrap()).unwrap();
    let _ = std::fs::remove_file(&l);
    turborepo_trace_core::os::symlink(target, l).unwrap();
}

const A_INDEX: &str = "import { u } from './util';\nimport b from 'b';\nimport _ from \
                       'lodash';\nimport './missing';\nconst fs = require('fs');\nconst loc = \
                       require('./locales/' + lang + '.json');\nfs.readFileSync(__dirname + \
                       '/data.json');\nexport const x = 1;\n";

/// A two-package workspace with a pnpm-style `node_modules` and a lockfile.
fn fixture() -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    let r = t.path();
    write(r, ".gitignore", "node_modules\n.turbotrace\n");
    write(
        r,
        "package.json",
        r#"{"name":"root","workspaces":["packages/*"]}"#,
    );
    write(r, "src/root.ts", "import a from 'a';\nexport default a;\n");
    write(
        r,
        "packages/a/package.json",
        r#"{"name":"a","main":"src/index.ts"}"#,
    );
    write(r, "packages/a/src/index.ts", A_INDEX);
    write(r, "packages/a/src/util.ts", "export const u = 1;\n");
    write(r, "packages/a/src/data.json", "{}");
    write(r, "packages/a/src/locales/en.json", "{}");
    write(
        r,
        "packages/b/package.json",
        r#"{"name":"b","main":"src/index.ts"}"#,
    );
    write(r, "packages/b/src/index.ts", "export default 2;\n");
    lodash(r, "4.17.21", "sha512-old");
    symlink(r, "../packages/a", "node_modules/a");
    symlink(r, "../../../packages/b", "packages/a/node_modules/b");
    symlink(
        r,
        "../../../node_modules/.pnpm/lodash@4.17.21/node_modules/lodash",
        "packages/a/node_modules/lodash",
    );
    t
}

/// Installs lodash `version` pnpm-style and writes the lockfile.
fn lodash(r: &Path, version: &str, integrity: &str) {
    let dir = format!("node_modules/.pnpm/lodash@{version}/node_modules/lodash");
    write(
        r,
        &format!("{dir}/package.json"),
        &format!(r#"{{"name":"lodash","version":"{version}","main":"index.js"}}"#),
    );
    write(
        r,
        &format!("{dir}/index.js"),
        "module.exports = require('./lib');\n",
    );
    write(r, &format!("{dir}/lib.js"), "module.exports = 1;\n");
    symlink(
        r,
        &format!("../../../node_modules/.pnpm/lodash@{version}/node_modules/lodash"),
        "packages/a/node_modules/lodash",
    );
    write(
        r,
        "pnpm-lock.yaml",
        &format!(
            "lockfileVersion: '9.0'\nimporters:\n  .: {{}}\n  packages/a:\n    dependencies:\n      lodash:\n        specifier: ^4\n        version: {version}\npackages:\n  lodash@{version}:\n    resolution: {{integrity: {integrity}}}\nsnapshots:\n  lodash@{version}: {{}}\n"
        ),
    );
}

fn opts(root: &Path, out: &Path, externals: Externals) -> GraphOptions {
    GraphOptions {
        root: root.to_path_buf(),
        out: Some(out.to_path_buf()),
        externals,
        threads: 2,
        ..Default::default()
    }
}

fn export(out: &Path) -> (String, Vec<String>) {
    let g = Graph::open(out).unwrap();
    let mut buf = Vec::new();
    turborepo_trace_graph::json::write_json(&g, Some(g.root()), true, &mut buf).unwrap();
    let keys = g
        .manifest()
        .shards
        .iter()
        .map(|s| format!("{} {} {}", s.kind, s.root, s.key))
        .collect();
    (String::from_utf8(buf).unwrap(), keys)
}

fn canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap()
}

#[test]
fn builds_composes_and_queries() {
    let t = fixture();
    let r = canonical(t.path());
    let out = r.join(".turbotrace");
    let rep = build(&opts(&r, &out, Externals::Opaque), &langs()).unwrap();
    assert_eq!(rep.first_party, 3, "root, a, b");
    assert_eq!(rep.third_party, 1, "lodash");
    assert_eq!(rep.reused, 0);
    let g = Graph::open(&out).unwrap();
    let paths: Vec<&str> = (0..g.node_count())
        .map(|i| g.path(turborepo_trace_graph::NodeId(i)))
        .collect();
    assert!(paths.contains(&"node_modules/lodash@4.17.21"), "{paths:?}");
    assert!(paths.contains(&"packages/a/src/data.json"), "{paths:?}");
    // Paths are sorted (dense ids follow path order).
    assert!(paths.windows(2).all(|w| w[0] < w[1]));

    let index = g.node_by_path("packages/a/src/index.ts").unwrap();
    let targets: Vec<&str> = g
        .out_edges(index)
        .filter_map(|e| e.to)
        .map(|n| g.path(n))
        .collect();
    for want in [
        "packages/a/src/util.ts",
        "packages/b/src/index.ts",
        "node_modules/lodash@4.17.21",
        "packages/a/src/locales/en.json",
        "packages/a/src/data.json",
        "packages/a/package.json",
    ] {
        assert!(targets.contains(&want), "{want} not in {targets:?}");
    }
    // Probes: extension probing for './missing' and the lodash walk-up are
    // recorded.
    let probes: Vec<&str> = g.probes(index).collect();
    assert!(probes.contains(&"packages/a/src/missing.ts"), "{probes:?}");
    assert!(probes.contains(&"packages/a/src/missing"), "{probes:?}");
    assert!(
        probes.contains(&"packages/a/src/node_modules/lodash"),
        "{probes:?}"
    );
    assert!(
        probes.contains(&"packages/a/src/package.json"),
        "{probes:?}"
    );
    // The glob is a membership with its precise pattern.
    let members: Vec<(String, String)> = g
        .memberships(index)
        .map(|m| (m.dir.to_string(), m.pattern.to_string()))
        .collect();
    assert!(
        members.contains(&("packages/a/src/locales".into(), "*.json".into())),
        "{members:?}"
    );
    let lodash = g.node_by_path("node_modules/lodash@4.17.21").unwrap();
    assert!(g.is_external(lodash));
    let info = g.package_info(g.package(lodash));
    assert_eq!(info.name, "lodash");
    assert_eq!(
        info.root,
        "node_modules/.pnpm/lodash@4.17.21/node_modules/lodash"
    );

    let engine = Engine::new(&g);
    let res = engine
        .query("rdeps(packages/b/src/index.ts) - packages/b/src/index.ts")
        .unwrap();
    let mut got: Vec<&str> = res.nodes.iter().map(|n| g.path(n)).collect();
    got.sort_unstable();
    assert_eq!(got, ["packages/a/src/index.ts", "src/root.ts"]);
    let res = engine
        .query("affected(added(packages/a/src/missing.ts), src/root.ts)")
        .unwrap();
    assert_eq!(res.affected.unwrap().entries.len(), 1);

    // A warm build reuses everything and composes the same graph.
    let before = export(&out);
    let rep = build(&opts(&r, &out, Externals::Opaque), &langs()).unwrap();
    assert_eq!(rep.traced, 0, "{rep:?}");
    assert_eq!(rep.analyzed_files, 0);
    assert_eq!(export(&out), before);

    // gc keeps exactly the referenced shards and the snapshot's walked-file list.
    std::fs::write(out.join("store/stale"), "x").unwrap();
    let gc = turborepo_trace_store::gc(&out).unwrap();
    assert_eq!(gc.removed, 1);
    assert_eq!(gc.kept, before.1.len() + 1);
}

#[test]
fn touching_one_file_retraces_one_shard() {
    let t = fixture();
    let r = canonical(t.path());
    let out = r.join(".turbotrace");
    build(&opts(&r, &out, Externals::Opaque), &langs()).unwrap();
    write(&r, "packages/b/src/index.ts", "export default 3;\n");
    let rep = build(&opts(&r, &out, Externals::Opaque), &langs()).unwrap();
    assert_eq!(rep.traced, 1, "{rep:?}");
    assert_eq!(rep.analyzed_files, 1);

    // A lockfile bump re-keys only the shard that links lodash (and adds its opaque
    // shard).
    lodash(&r, "4.17.22", "sha512-new");
    let rep = build(&opts(&r, &out, Externals::Opaque), &langs()).unwrap();
    assert_eq!(
        rep.traced, 2,
        "package a and the new opaque lodash: {rep:?}"
    );
    // Unchanged files of the re-traced package reuse their extraction.
    assert_eq!(rep.reused_extractions, rep.analyzed_files, "{rep:?}");
}

#[test]
fn follow_traces_third_party_shards() {
    let t = fixture();
    let r = canonical(t.path());
    let out = r.join(".turbotrace");
    let rep = build(&opts(&r, &out, Externals::Follow), &langs()).unwrap();
    assert_eq!(rep.third_party, 1);
    let g = Graph::open(&out).unwrap();
    let lib = "node_modules/.pnpm/lodash@4.17.21/node_modules/lodash/lib.js";
    let index = g
        .node_by_path("node_modules/.pnpm/lodash@4.17.21/node_modules/lodash/index.js")
        .unwrap();
    assert!(g.is_external(index));
    let targets: Vec<&str> = g
        .out_edges(index)
        .filter_map(|e| e.to)
        .map(|n| g.path(n))
        .collect();
    assert!(targets.contains(&lib), "{targets:?}");
    let before = export(&out);
    let rep = build(&opts(&r, &out, Externals::Follow), &langs()).unwrap();
    assert_eq!(rep.traced, 0, "{rep:?}");
    assert_eq!(export(&out), before);
}

#[test]
fn lockfile_only_packages_without_install() {
    let t = fixture();
    let r = canonical(t.path());
    std::fs::remove_file(r.join("packages/a/node_modules/lodash")).unwrap();
    let out = r.join(".turbotrace");
    let rep = build(&opts(&r, &out, Externals::Opaque), &langs()).unwrap();
    assert_eq!(rep.third_party, 1, "{rep:?}");
    let g = Graph::open(&out).unwrap();
    let index = g.node_by_path("packages/a/src/index.ts").unwrap();
    let targets: Vec<&str> = g
        .out_edges(index)
        .filter_map(|e| e.to)
        .map(|n| g.path(n))
        .collect();
    assert!(
        targets.contains(&"node_modules/lodash@4.17.21"),
        "{targets:?}"
    );
    // No unresolved-specifier uncertainty for it.
    assert!(
        !g.uncertainties(index)
            .any(|u| u.detail.contains("'lodash'"))
    );
    // Installing it later re-keys package a (the walk-up probe matches).
    lodash(&r, "4.17.21", "sha512-old");
    let rep = build(&opts(&r, &out, Externals::Opaque), &langs()).unwrap();
    assert_eq!(rep.traced, 1, "{rep:?}");
}

/// One random edit.
#[derive(Clone, Debug)]
enum Edit {
    Write(usize, Vec<usize>),
    Delete(usize),
    BumpLodash(u8),
    AddPackage,
    Tsconfig(bool),
    Uninstall,
}

const FILES: &[&str] = &[
    "packages/a/src/index.ts",
    "packages/a/src/util.ts",
    "packages/a/src/missing.ts",
    "packages/a/src/dir/index.ts",
    "packages/a/src/locales/de.json",
    "packages/a/src/data.json",
    "packages/b/src/index.ts",
    "packages/b/src/other.ts",
    "src/root.ts",
    "packages/c/src/c.ts",
];

const LINES: &[&str] = &[
    "import { u } from './util';",
    "import './missing';",
    "import _ from 'lodash';",
    "import b from 'b';",
    "import d from './dir';",
    "const l = require('./locales/' + x + '.json');",
    "require('fs').readFileSync(__dirname + '/data.json');",
    "import o from '../../b/src/other';",
    "import a from 'a';",
    "import p from '@/util';",
    "import type { T } from './types';",
    "const w = require(name);",
];

fn edit() -> impl Strategy<Value = Edit> {
    prop_oneof![
        4 => (0..FILES.len(), proptest::collection::vec(0..LINES.len(), 0..4))
            .prop_map(|(f, l)| Edit::Write(f, l)),
        2 => (0..FILES.len()).prop_map(Edit::Delete),
        1 => (0u8..3).prop_map(Edit::BumpLodash),
        1 => Just(Edit::AddPackage),
        1 => any::<bool>().prop_map(Edit::Tsconfig),
        1 => Just(Edit::Uninstall),
    ]
}

fn apply(r: &Path, e: &Edit) {
    match e {
        Edit::Write(f, lines) => {
            let mut s: String = lines.iter().map(|&l| format!("{}\n", LINES[l])).collect();
            s.push_str("export default 1;\n");
            write(r, FILES[*f], &s);
        }
        Edit::Delete(f) => {
            let _ = std::fs::remove_file(r.join(FILES[*f]));
        }
        Edit::BumpLodash(v) => lodash(r, &format!("4.17.{}", 30 + v), &format!("sha512-{v}")),
        Edit::AddPackage => write(r, "packages/c/package.json", r#"{"name":"c"}"#),
        Edit::Uninstall => {
            let _ = std::fs::remove_file(r.join("packages/a/node_modules/lodash"));
        }
        Edit::Tsconfig(on) => {
            if *on {
                write(
                    r,
                    "packages/a/tsconfig.json",
                    r#"{"compilerOptions":{"baseUrl":".","paths":{"@/*":["src/*"]}}}"#,
                );
            } else {
                let _ = std::fs::remove_file(r.join("packages/a/tsconfig.json"));
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 24, ..ProptestConfig::default() })]

    #[test]
    fn composed_equals_fresh(edits in proptest::collection::vec(edit(), 1..5)) {
        let t = fixture();
        let r = canonical(t.path());
        let out = r.join(".turbotrace");
        build(&opts(&r, &out, Externals::Opaque), &langs()).unwrap();
        for e in &edits {
            apply(&r, e);
            build(&opts(&r, &out, Externals::Opaque), &langs()).unwrap();
            let fresh = tempfile::tempdir().unwrap();
            build(&opts(&r, fresh.path(), Externals::Opaque), &langs()).unwrap();
            let (inc_json, inc_keys) = export(&out);
            let (fresh_json, fresh_keys) = export(fresh.path());
            if inc_json != fresh_json {
                let diff: Vec<String> = inc_json
                    .lines()
                    .zip(fresh_json.lines())
                    .enumerate()
                    .filter(|(_, (a, b))| a != b)
                    .take(6)
                    .map(|(i, (a, b))| format!("{i}: incremental {a} | fresh {b}"))
                    .collect();
                prop_assert!(false, "graph differs after {:?}:\n{}", e, diff.join("\n"));
            }
            prop_assert_eq!(inc_keys, fresh_keys, "after {:?}", e);
        }
    }
}

// ---- Mixed-language repository ----

/// A repository with a JS, a Python, a Go and a Rust package, plus environments
/// outside it: a Python site directory, a Go module cache and a cargo home
/// (sibling directories of the repository, as on a developer machine or CI).
struct Mixed {
    _t: tempfile::TempDir,
    repo: PathBuf,
    env: PathBuf,
}

impl Mixed {
    fn site(&self) -> PathBuf {
        self.env.join("venv/lib/python3.12/site-packages")
    }
    fn modcache(&self) -> PathBuf {
        self.env.join("gomod")
    }
    fn cargo_home(&self) -> PathBuf {
        self.env.join("cargo")
    }

    fn langs(&self) -> Vec<Arc<dyn Language>> {
        use turborepo_trace_lang_go::{GoLanguage, GoOptions, GoRoot};
        vec![
            Arc::new(turborepo_trace_lang_js::JsLanguage),
            Arc::new(GoLanguage::new(GoOptions {
                modcache: Some(self.modcache()),
                goroot: GoRoot::None,
                cgo: Some(false),
                workspace: Some(false),
                ..GoOptions::default()
            })),
            Arc::new(turborepo_trace_lang_rust::RustLanguage::with_cargo_home(
                self.cargo_home(),
            )),
        ]
    }

    /// Installs `requests` `version` into the site directory (in place).
    fn requests(&self, version: &str) {
        let site = self.site();
        if let Ok(rd) = std::fs::read_dir(&site) {
            for e in rd.flatten() {
                if e.file_name().to_string_lossy().starts_with("requests-") {
                    std::fs::remove_dir_all(e.path()).unwrap();
                }
            }
        }
        write(
            &site,
            "requests/__init__.py",
            &format!("__version__ = '{version}'\nfrom . import api\n"),
        );
        write(&site, "requests/api.py", "def get(url): pass\n");
        let di = format!("requests-{version}.dist-info");
        write(
            &site,
            &format!("{di}/METADATA"),
            &format!("Name: requests\nVersion: {version}\n"),
        );
        write(
            &site,
            &format!("{di}/RECORD"),
            &format!("requests/__init__.py,,\nrequests/api.py,,\n{di}/METADATA,,\n"),
        );
        write(&site, &format!("{di}/top_level.txt"), "requests\n");
    }

    /// Requires `golang.org/x/text` `version` and puts it in the module cache.
    fn x_text(&self, version: &str) {
        let dir = format!("golang.org/x/text@{version}");
        write(
            &self.modcache(),
            &format!("{dir}/go.mod"),
            "module golang.org/x/text\n\ngo 1.18\n",
        );
        write(
            &self.modcache(),
            &format!("{dir}/language/language.go"),
            "package language\n\nfunc Make(s string) string { return s }\n",
        );
        write(
            &self.repo,
            "svc/go.mod",
            &format!("module example.com/svc\n\ngo 1.22\n\nrequire golang.org/x/text {version}\n"),
        );
        write(
            &self.repo,
            "svc/go.sum",
            &format!(
                "golang.org/x/text {version} h1:sum-{version}=\ngolang.org/x/text \
                 {version}/go.mod h1:mod=\n"
            ),
        );
    }

    /// Locks `serde` `version` and puts it in the registry.
    fn serde(&self, version: &str) {
        let reg = self
            .cargo_home()
            .join("registry/src/index.crates.io-1949cf8c6b5b557f");
        write(
            &reg,
            &format!("serde-{version}/Cargo.toml"),
            &format!("[package]\nname = \"serde\"\nversion = \"{version}\"\nedition = \"2021\"\n"),
        );
        write(
            &reg,
            &format!("serde-{version}/src/lib.rs"),
            "pub trait Serialize {}\n",
        );
        write(
            &self.repo,
            "rs/Cargo.lock",
            &format!(
                "version = 3\n\n[[package]]\nname = \"rsapp\"\nversion = \"0.1.0\"\ndependencies = [\n \"serde\",\n]\n\n[[package]]\nname = \"serde\"\nversion = \"{version}\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\nchecksum = \"c-{version}\"\n"
            ),
        );
    }
}

fn mixed() -> Mixed {
    let t = tempfile::tempdir().unwrap();
    let base = canonical(t.path());
    let repo = base.join("repo");
    let env = base.join("env");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(&env).unwrap();
    let m = Mixed {
        _t: t,
        repo: repo.clone(),
        env,
    };
    let r = &repo;
    write(r, ".gitignore", ".turbotrace\n");
    write(r, "package.json", r#"{"name":"root","workspaces":["web"]}"#);
    write(r, "web/package.json", r#"{"name":"web","main":"index.ts"}"#);
    write(
        r,
        "web/index.ts",
        "import { h } from './h';\nexport default h;\n",
    );
    write(r, "web/h.ts", "export const h = 1;\n");
    write(
        r,
        "py/pyproject.toml",
        "[project]\nname = \"PyApp\"\nversion = \"0.1.0\"\n",
    );
    write(r, "py/app/__init__.py", "");
    write(
        r,
        "py/app/main.py",
        "import requests\nfrom . import util\nfrom .util import helper\n",
    );
    write(r, "py/app/util.py", "def helper(): pass\n");
    write(
        r,
        "svc/main.go",
        "package main\n\nimport \
         (\n\t\"example.com/svc/lib\"\n\t\"golang.org/x/text/language\"\n)\n\nfunc main() { \
         lib.Do(language.Make(\"en\")) }\n",
    );
    write(r, "svc/lib/lib.go", "package lib\n\nfunc Do(s string) {}\n");
    write(
        r,
        "rs/Cargo.toml",
        "[package]\nname = \"rsapp\"\nversion = \"0.1.0\"\nedition = \
         \"2021\"\n\n[dependencies]\nserde = \"1\"\n",
    );
    write(
        r,
        "rs/src/main.rs",
        "mod util;\nfn main() { util::run(); }\n",
    );
    write(r, "rs/src/util.rs", "pub fn run() {}\n");
    m.requests("2.31.0");
    m.x_text("v0.14.0");
    m.serde("1.0.200");
    m
}

fn targets_of(g: &Graph, path: &str) -> Vec<String> {
    let n = g
        .node_by_path(path)
        .unwrap_or_else(|| panic!("no node {path}"));
    g.out_edges(n)
        .filter_map(|e| e.to)
        .map(|n| g.path(n).to_owned())
        .collect()
}

#[test]
fn mixed_language_graph() {
    let m = mixed();
    let out = m.repo.join(".turbotrace");
    let rep = build(&opts(&m.repo, &out, Externals::Opaque), &m.langs()).unwrap();
    let g = Graph::open(&out).unwrap();
    let mut shards: Vec<(String, String)> = g
        .manifest()
        .shards
        .iter()
        .map(|s| (s.kind.clone(), s.name.clone()))
        .collect();
    shards.sort();
    // The Python project is a workspace package with or without the language; only
    // the language reaches the distribution it imports.
    let want: Vec<(String, String)> = vec![
        ("first-party".into(), "crates:rsapp".into()),
        ("first-party".into(), "go:example.com/svc".into()),
        ("first-party".into(), "pypi:pyapp".into()),
        ("first-party".into(), "root".into()),
        ("first-party".into(), "web".into()),
        ("opaque".into(), "crates:serde".into()),
        ("opaque".into(), "go:golang.org/x/text".into()),
    ];
    assert_eq!(shards, want, "{rep:?}");
    // Go: the package in the same module and the module from the module cache.
    let go = targets_of(&g, "svc/main.go");
    for want in [
        "svc/lib/lib.go",
        "go:golang.org/x/text@v0.14.0",
        "svc/go.mod",
    ] {
        assert!(go.iter().any(|t| t == want), "{want} not in {go:?}");
    }
    // Rust: the module file and the registry crate.
    let rs = targets_of(&g, "rs/src/main.rs");
    for want in ["rs/src/util.rs", "crates:serde@1.0.200", "rs/Cargo.toml"] {
        assert!(rs.iter().any(|t| t == want), "{want} not in {rs:?}");
    }
    let serde = g.node_by_path("crates:serde@1.0.200").unwrap();
    assert!(g.is_external(serde));
    let info = g.package_info(g.package(serde));
    assert_eq!(info.name, "crates:serde");
    // Integrity per ecosystem: Cargo.lock checksum, go.sum hash.
    let key_of = |name: &str| {
        g.manifest()
            .shards
            .iter()
            .find(|s| s.name == name)
            .map(|s| s.key.clone())
            .unwrap()
    };
    // Lockfiles of every ecosystem are recorded.
    let locks: Vec<&str> = g
        .manifest()
        .lockfiles
        .iter()
        .map(|l| l.path.as_str())
        .collect();
    assert_eq!(locks, ["rs/Cargo.lock", "svc/go.sum"]);

    // Warm rebuild reuses everything.
    let before = export(&out);
    let rep = build(&opts(&m.repo, &out, Externals::Opaque), &m.langs()).unwrap();
    assert_eq!(rep.traced, 0, "{rep:?}");
    assert_eq!(export(&out), before);

    // Bumps re-key exactly the linking shard (plus the new opaque shard).
    let svc = key_of("go:example.com/svc");
    let rs_key = key_of("crates:rsapp");
    m.serde("1.0.201");
    let rep = build(&opts(&m.repo, &out, Externals::Opaque), &m.langs()).unwrap();
    assert_eq!(rep.traced, 2, "rsapp and the new serde: {rep:?}");
    let g = Graph::open(&out).unwrap();
    assert!(g.node_by_path("crates:serde@1.0.201").is_some());
    assert_ne!(
        g.manifest()
            .shards
            .iter()
            .find(|s| s.name == "crates:rsapp")
            .unwrap()
            .key,
        rs_key
    );
    assert_eq!(
        g.manifest()
            .shards
            .iter()
            .find(|s| s.name == "go:example.com/svc")
            .unwrap()
            .key,
        svc
    );
    m.requests("2.32.0");
    let rep = build(&opts(&m.repo, &out, Externals::Opaque), &m.langs()).unwrap();
    // (Without the language nothing reaches the site directory.)
    let traced = 0;
    assert_eq!(rep.traced, traced, "pyapp and the new requests: {rep:?}");
    m.x_text("v0.15.0");
    let rep = build(&opts(&m.repo, &out, Externals::Opaque), &m.langs()).unwrap();
    assert_eq!(rep.traced, 2, "svc and the new x/text: {rep:?}");
}

/// One random edit of the mixed repository.
#[derive(Clone, Debug)]
enum MixedEdit {
    Write(usize, Vec<usize>),
    Delete(usize),
    Requests(u8),
    XText(u8),
    Serde(u8),
}

const MIXED_FILES: &[&str] = &[
    "py/app/main.py",
    "py/app/util.py",
    "py/app/extra.py",
    "svc/main.go",
    "svc/lib/lib.go",
    "svc/lib/more.go",
    "rs/src/main.rs",
    "rs/src/util.rs",
    "rs/src/extra.rs",
    // What could come to provide the module of `import missing_module`: a module file next
    // to the importer, a package on the project's search path, a file of that name
    // elsewhere in the repository. An unresolved Python import is covered by the places a
    // module of its name could appear, so each of them re-traces the importers.
    "py/app/missing_module.py",
    "py/missing_module/__init__.py",
    "tools/missing_module.py",
];

/// Lines per language: (python, go, rust).
const MIXED_LINES: &[(&str, &str, &str)] = &[
    (
        "import requests",
        "import \"golang.org/x/text/language\"",
        "mod util;",
    ),
    (
        "from . import util",
        "import \"example.com/svc/lib\"",
        "mod extra;",
    ),
    (
        "from .extra import x",
        "import \"os\"",
        "use serde::Serialize;",
    ),
    (
        "import missing_module",
        "import \"example.com/svc/missing\"",
        "mod missing;",
    ),
    (
        "open(__file__ + '.txt')",
        "//go:embed data.txt",
        "const D: &str = include_str!(\"data.txt\");",
    ),
];

fn mixed_edit() -> impl Strategy<Value = MixedEdit> {
    prop_oneof![
        4 => (0..MIXED_FILES.len(), proptest::collection::vec(0..MIXED_LINES.len(), 0..3))
            .prop_map(|(f, l)| MixedEdit::Write(f, l)),
        2 => (0..MIXED_FILES.len()).prop_map(MixedEdit::Delete),
        1 => (0u8..2).prop_map(MixedEdit::Requests),
        1 => (0u8..2).prop_map(MixedEdit::XText),
        1 => (0u8..2).prop_map(MixedEdit::Serde),
    ]
}

fn apply_mixed(m: &Mixed, e: &MixedEdit) {
    match e {
        MixedEdit::Write(f, lines) => {
            let file = MIXED_FILES[*f];
            let s: String = if file.ends_with(".py") {
                lines
                    .iter()
                    .map(|&l| format!("{}\n", MIXED_LINES[l].0))
                    .collect()
            } else if file.ends_with(".go") {
                let pkg = if file.starts_with("svc/lib") {
                    "lib"
                } else {
                    "main"
                };
                let mut s = format!("package {pkg}\n\n");
                for &l in lines {
                    s.push_str(MIXED_LINES[l].1);
                    s.push('\n');
                }
                s
            } else {
                lines
                    .iter()
                    .map(|&l| format!("{}\n", MIXED_LINES[l].2))
                    .collect()
            };
            write(&m.repo, file, &s);
        }
        MixedEdit::Delete(f) => {
            let _ = std::fs::remove_file(m.repo.join(MIXED_FILES[*f]));
        }
        MixedEdit::Requests(v) => m.requests(&format!("2.{}.0", 40 + v)),
        MixedEdit::XText(v) => m.x_text(&format!("v0.{}.0", 20 + v)),
        MixedEdit::Serde(v) => m.serde(&format!("1.0.{}", 300 + u32::from(*v))),
    }
}

/// The cases of [`mixed_composed_equals_fresh`]: 12, or
/// `TURBO_TRACE_DEBUG_MIXED_CASES` for a gate run.
fn mixed_cases() -> u32 {
    std::env::var("TURBO_TRACE_DEBUG_MIXED_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(12)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: mixed_cases(), ..ProptestConfig::default() })]

    #[test]
    fn mixed_composed_equals_fresh(edits in proptest::collection::vec(mixed_edit(), 1..4)) {
        let m = mixed();
        let out = m.repo.join(".turbotrace");
        let langs = m.langs();
        build(&opts(&m.repo, &out, Externals::Opaque), &langs).unwrap();
        for e in &edits {
            apply_mixed(&m, e);
            build(&opts(&m.repo, &out, Externals::Opaque), &m.langs()).unwrap();
            let fresh = tempfile::tempdir().unwrap();
            build(&opts(&m.repo, fresh.path(), Externals::Opaque), &m.langs()).unwrap();
            let (inc_json, inc_keys) = export(&out);
            let (fresh_json, fresh_keys) = export(fresh.path());
            if inc_json != fresh_json {
                let diff: Vec<String> = inc_json
                    .lines()
                    .zip(fresh_json.lines())
                    .enumerate()
                    .filter(|(_, (a, b))| a != b)
                    .take(6)
                    .map(|(i, (a, b))| format!("{i}: incremental {a} | fresh {b}"))
                    .collect();
                prop_assert!(false, "graph differs after {:?}:\n{}", e, diff.join("\n"));
            }
            prop_assert_eq!(inc_keys, fresh_keys, "after {:?}", e);
        }
    }
}

// ---- The composed graph reused across an edit ----

/// Opens the graph in `out` with its tables, waits for the cache files, and
/// says whether the composed parts were the previous snapshot's.
fn open_warm(out: &Path) -> bool {
    let g = Graph::open(out).unwrap();
    let reused = g.load_stats().reused;
    g.warm();
    drop(g);
    turborepo_trace_store::wait_for_cache_writes();
    reused
}

/// The incrementally refreshed graph in `out` against a fresh build of `r`: the
/// export, both cache files byte for byte, and the derived mock layer
/// ([`mocks_same`]). Returns the number of edges of that layer and whether the
/// cache answered the first read of it.
fn same_as_fresh(r: &Path, out: &Path) -> Result<(usize, bool), String> {
    let fresh = tempfile::tempdir().unwrap();
    build(&opts(r, fresh.path(), Externals::Opaque), &langs()).unwrap();
    if open_warm(fresh.path()) {
        return Err("a fresh build reused something".into());
    }
    if export(out) != export(fresh.path()) {
        return Err("the export differs from a fresh build's".into());
    }
    for name in ["cache/composed", "cache/tables"] {
        if std::fs::read(out.join(name)).unwrap() != std::fs::read(fresh.path().join(name)).unwrap()
        {
            return Err(format!("{name} differs from a fresh build's"));
        }
    }
    mocks_same(out, fresh.path())
}

/// The derived mock layer (`Graph::mock_patch`: the edges from modules to the
/// manual mocks that helpers register, and the tags of the files with markers)
/// of the incrementally refreshed graph in `out` against the one of the fresh
/// build in `fresh`: the layer the store's cache answers with is the layer
/// computed from the graph as it is now, both equal the fresh build's byte for
/// byte, and so does the cache file. Returns the number of edges and whether
/// the first read came from the cache.
fn mocks_same(out: &Path, fresh: &Path) -> Result<(usize, bool), String> {
    let layer = |dir: &Path| {
        let g = Graph::open(dir).unwrap();
        let (cached, n, hit) = g.mock_patch_cached();
        let (computed, m) = g.mock_patch();
        (cached.to_bytes(), computed.to_bytes(), n, m, hit)
    };
    let (cached, computed, n, m, hit) = layer(out);
    if cached != computed || n != m {
        return Err(format!(
            "the cached mock layer (read from the cache: {hit}) is not the graph's"
        ));
    }
    let (fresh_cached, fresh_computed, ..) = layer(fresh);
    if computed != fresh_computed || cached != fresh_cached {
        return Err("the mock layer differs from a fresh build's".into());
    }
    // A second read is answered by the cache, with the same layer.
    let (again, _, _, _, hit_again) = layer(out);
    if !hit_again || again != cached {
        return Err("the mock layer was not kept in the cache".into());
    }
    let name = "cache/derived-mocks";
    if std::fs::read(out.join(name)).unwrap() != std::fs::read(fresh.join(name)).unwrap() {
        return Err(format!("{name} differs from a fresh build's"));
    }
    Ok((n, hit))
}

/// A derived layer across an edit ([`Graph::carried_patch`]): when the composed
/// graph was kept, the layer's owner is offered the stored layer of the
/// snapshot before with the files whose contents changed, and what it accepts
/// is this graph's layer from then on.
#[test]
fn a_derived_layer_is_carried_across_an_edit_when_its_owner_says_so() {
    use turborepo_trace_graph::{NodeId, Patch};
    use turborepo_trace_store::Kept;
    let t = fixture();
    let r = canonical(t.path());
    let out = r.join(".turbotrace");
    let rebuild = || build(&opts(&r, &out, Externals::Opaque), &langs()).unwrap();
    // The layer of a graph: how it came about, its numbers, and what its owner was
    // asked.
    let layer = |g: &Graph, key: u128, say: bool| {
        let mut asked = None;
        let (_, meta, kept) = g.carried_patch(
            "test-layer",
            key,
            |meta, changed| {
                let paths: Vec<String> = changed.iter().map(|n| g.path(*n).to_owned()).collect();
                asked = Some((meta.to_vec(), paths));
                say
            },
            || {
                let mut p = Patch::new();
                p.tag(NodeId(0), "computed");
                (p, vec![g.node_count() as u64, 42])
            },
        );
        (kept, meta, asked)
    };
    let settle = |g: Graph| {
        g.warm();
        drop(g);
        turborepo_trace_store::wait_for_cache_writes();
    };
    rebuild();
    let g = Graph::open(&out).unwrap();
    assert!(g.carried().is_none(), "composed from the shards");
    let nodes = g.node_count() as u64;
    assert_eq!(layer(&g, 7, true), (Kept::Computed, vec![nodes, 42], None));
    assert_eq!(layer(&g, 7, true), (Kept::Cached, vec![nodes, 42], None));
    settle(g);

    // An edit that keeps every reference: the composed graph is kept, and the graph
    // names the file that changed. An owner that read it computes again.
    let index = "packages/a/src/index.ts";
    write(&r, index, &format!("// one more line\n\n{A_INDEX}"));
    rebuild();
    let g = Graph::open(&out).unwrap();
    assert!(g.load_stats().reused);
    let changed: Vec<&str> = g.carried().unwrap().iter().map(|n| g.path(*n)).collect();
    assert_eq!(changed, [index]);
    let asked = Some((vec![nodes, 42], vec![index.to_owned()]));
    assert_eq!(
        layer(&g, 7, false),
        (Kept::Computed, vec![nodes, 42], asked.clone())
    );
    assert_eq!(layer(&g, 7, true), (Kept::Cached, vec![nodes, 42], None));
    settle(g);

    // Another such edit, and an owner that did not read the file: the stored layer
    // is this graph's, and is stored as such. A layer with other inputs is not
    // offered.
    write(
        &r,
        index,
        &format!("// one more line\n\n{A_INDEX}export const y = 2;\n"),
    );
    write(&r, "packages/b/src/index.ts", "export default 3;\n");
    rebuild();
    let g = Graph::open(&out).unwrap();
    let changed: Vec<&str> = g.carried().unwrap().iter().map(|n| g.path(*n)).collect();
    assert_eq!(changed, [index, "packages/b/src/index.ts"]);
    let asked = Some((
        vec![nodes, 42],
        vec![index.to_owned(), "packages/b/src/index.ts".to_owned()],
    ));
    assert_eq!(layer(&g, 7, true), (Kept::Carried, vec![nodes, 42], asked));
    assert_eq!(layer(&g, 7, false), (Kept::Cached, vec![nodes, 42], None));
    settle(g);
    // The same manifest again: nothing was carried, and the layer is there.
    let g = Graph::open(&out).unwrap();
    assert!(g.carried().is_none());
    assert_eq!(layer(&g, 7, false), (Kept::Cached, vec![nodes, 42], None));
    settle(g);

    // A layer with other inputs of its own (another key) is not the stored one: its
    // owner is not asked.
    write(&r, index, &format!("// two more lines\n\n\n{A_INDEX}"));
    rebuild();
    let g = Graph::open(&out).unwrap();
    assert!(g.carried().is_some());
    assert_eq!(layer(&g, 8, true), (Kept::Computed, vec![nodes, 42], None));
    settle(g);

    // An edit that composition sees (a new reference): composed again, nothing to
    // carry.
    write(
        &r,
        "packages/b/src/index.ts",
        "import './other';\nexport default 3;\n",
    );
    rebuild();
    let g = Graph::open(&out).unwrap();
    assert!(!g.load_stats().reused && g.carried().is_none());
    let (kept, _, asked) = layer(&g, 8, true);
    assert_eq!((kept, asked), (Kept::Computed, None));
}

#[test]
fn an_edit_that_keeps_references_reuses_the_composed_graph() {
    let t = fixture();
    let r = canonical(t.path());
    let out = r.join(".turbotrace");
    let rebuild = || build(&opts(&r, &out, Externals::Opaque), &langs()).unwrap();
    rebuild();
    assert!(!open_warm(&out), "the first open composes");
    assert!(
        !open_warm(&out),
        "an unchanged manifest reads its own cache"
    );

    // A comment above every reference of a file: the shard's key and every line
    // change.
    write(
        &r,
        "packages/a/src/index.ts",
        &format!("// one more line\n\n{A_INDEX}"),
    );
    assert_eq!(rebuild().traced, 1);
    assert!(
        open_warm(&out),
        "the same references: composed parts reused"
    );
    same_as_fresh(&r, &out).unwrap();
    // A body edit below them, and one in another package at once.
    write(
        &r,
        "packages/a/src/index.ts",
        &format!("// one more line\n\n{A_INDEX}export const y = 2;\n"),
    );
    write(&r, "packages/b/src/index.ts", "export default 3;\n");
    assert_eq!(rebuild().traced, 2);
    assert!(open_warm(&out));
    same_as_fresh(&r, &out).unwrap();

    // One more reference: composed again.
    write(
        &r,
        "packages/b/src/index.ts",
        "import './other';\nexport default 3;\n",
    );
    rebuild();
    assert!(!open_warm(&out), "a new reference");
    same_as_fresh(&r, &out).unwrap();
    // A new file (another walked-file list): composed again, and after it reuse
    // works.
    write(&r, "packages/b/src/other.ts", "export {};\n");
    rebuild();
    assert!(!open_warm(&out), "a new file");
    write(&r, "packages/b/src/other.ts", "// same\nexport {};\n");
    rebuild();
    assert!(open_warm(&out));
    same_as_fresh(&r, &out).unwrap();
    // Without the shard that was replaced (the store was garbage-collected after
    // the refresh) there is nothing to compare with.
    write(&r, "packages/b/src/other.ts", "// same again\nexport {};\n");
    rebuild();
    turborepo_trace_store::gc(&out).unwrap();
    assert!(!open_warm(&out), "the replaced shard is gone");
    same_as_fresh(&r, &out).unwrap();
}

// ---- Mock calls across edits: the markers, the reuse and the derived mock
// layer ----

const MOCK_TEST: &str = "import { u } from '../packages/a/src/util';\nimport { db } from \
                         '../packages/b/src/db';\ntest('a', () => { u; db; });\n";
const MOCK_HELPER: &str = "export const h = 1;\n";
const FACTORY: &str = "jest.mock('../packages/a/src/util', () => ({ u: 2 }));";
const BARE: &str = "jest.mock('../packages/b/src/db');";

/// The files the mock steps edit: two tests, a helper a test imports, the
/// configured setup file and a helper of its closure.
const MOCK_FILES: &[&str] = &[
    "test/a.test.ts",
    "test/b.test.ts",
    "test/helper.ts",
    "test/setup.ts",
    "test/env.ts",
];

/// Calls of the mock APIs, one per line. Each form leaves another marker (a
/// replacement with its line, a manual mock, a real load, a hazard with its
/// line) or none.
const MOCK_CALLS: &[&str] = &[
    FACTORY,
    BARE,
    "jest.mock('../packages/b/src/db', () => ({ db: 3 }));",
    "jest.requireActual('../packages/a/src/util');",
    "jest.mock(name);",
    "jest.doMock('../packages/b/src/db');",
    "jest.unmock('../packages/a/src/util');",
];

/// [`fixture`] with a jest project at the root: two tests that import a module
/// of each package, a helper one of them imports, a setup file and a helper of
/// its closure, and a manual mock next to `packages/b/src/db.ts`. No file calls
/// a mock API yet.
fn mock_fixture() -> tempfile::TempDir {
    let t = fixture();
    let r = t.path();
    write(
        r,
        "jest.config.js",
        "module.exports = { testMatch: ['**/test/**/*.test.ts'], setupFilesAfterEnv: \
         ['<rootDir>/test/setup.ts'] };\n",
    );
    write(r, "packages/b/src/db.ts", "export const db = 1;\n");
    write(
        r,
        "packages/b/src/__mocks__/db.ts",
        "export const db = 2;\n",
    );
    write(
        r,
        "test/a.test.ts",
        &format!("import './helper';\n{MOCK_TEST}"),
    );
    write(r, "test/b.test.ts", MOCK_TEST);
    write(r, "test/helper.ts", MOCK_HELPER);
    write(r, "test/setup.ts", "import './env';\n");
    write(r, "test/env.ts", "export const e = 1;\n");
    t
}

/// The marker edges of `file` in the graph in `out`, sorted.
fn markers(out: &Path, file: &str) -> Vec<String> {
    let g = Graph::open(out).unwrap();
    let n = g.node_by_path(file).unwrap();
    let mut v: Vec<String> = g
        .out_edges(n)
        .filter_map(|e| e.specifier.filter(|s| s.starts_with("mock:")))
        .map(str::to_owned)
        .collect();
    v.sort();
    v
}

/// A mock call added to a file, removed from it or moved in it changes the
/// file's marker edges. Composition sees every change of a marker, so the
/// composed graph is not reused across one; where a move leaves the markers as
/// they are (a marker that names no line), it is reused. The derived mock layer
/// is computed again for every new manifest, from the graph as it is then.
/// After each step everything equals a fresh build.
#[test]
fn mock_calls_and_the_composed_graph() {
    let t = mock_fixture();
    let r = canonical(t.path());
    let out = r.join(".turbotrace");
    let rebuild = || build(&opts(&r, &out, Externals::Opaque), &langs()).unwrap();
    // Everything equals a fresh build; the edges of the mock layer. The first read
    // of the layer after a refresh is never answered by the cache: the manifest
    // is new.
    let check = |what: &str| -> usize {
        let (n, hit) = same_as_fresh(&r, &out).unwrap_or_else(|e| panic!("{what}: {e}"));
        assert!(!hit, "{what}: the layer of another manifest answered");
        n
    };
    rebuild();
    assert!(!open_warm(&out), "the first open composes");
    assert_eq!(check("no mock call"), 0);
    assert!(markers(&out, "test/a.test.ts").is_empty());

    // Added to a test: a replacement marker (it names the call's line).
    write(
        &r,
        "test/a.test.ts",
        &format!("import './helper';\n{MOCK_TEST}{FACTORY}\n"),
    );
    assert_eq!(rebuild().traced, 1);
    assert!(!open_warm(&out), "a new marker: composed again");
    assert_eq!(check("a factory mock added to a test"), 0);
    assert_eq!(
        markers(&out, "test/a.test.ts"),
        ["mock:replaces jest path packages/a/src/util.ts @ test/a.test.ts:5"]
    );
    // Moved in the test (a line above it): the marker names another line.
    write(
        &r,
        "test/a.test.ts",
        &format!("import './helper';\n// moved\n{MOCK_TEST}{FACTORY}\n"),
    );
    rebuild();
    assert!(!open_warm(&out), "the marker changed: composed again");
    check("a factory mock moved in a test");
    assert_eq!(
        markers(&out, "test/a.test.ts"),
        ["mock:replaces jest path packages/a/src/util.ts @ test/a.test.ts:6"]
    );
    // An edit below the call keeps the marker: reused, and the layer is this
    // manifest's.
    write(
        &r,
        "test/a.test.ts",
        &format!("import './helper';\n// moved\n{MOCK_TEST}{FACTORY}\ntest('more', () => {{}});\n"),
    );
    rebuild();
    assert!(open_warm(&out), "the same markers: composed parts reused");
    check("an edit below the mock call");

    // Added to a helper: the marker of a manual mock, which the derived layer turns
    // into an edge from the module to the mock.
    write(&r, "test/helper.ts", &format!("{BARE}\n{MOCK_HELPER}"));
    rebuild();
    assert!(!open_warm(&out), "a new marker: composed again");
    assert_eq!(check("a manual mock registered by a helper"), 1);
    let manual = "mock:manual jest packages/b/src/__mocks__/db.ts for packages/b/src/db.ts";
    assert_eq!(markers(&out, "test/helper.ts"), [manual]);
    // Moved in the helper: this marker names no line, so nothing that composition
    // reads changed. The composed parts are reused and the new manifest's layer
    // has the edge.
    write(&r, "test/helper.ts", &format!("{MOCK_HELPER}\n{BARE}\n"));
    rebuild();
    assert!(open_warm(&out), "the same marker: composed parts reused");
    assert_eq!(check("a helper's manual mock moved"), 1);
    assert_eq!(markers(&out, "test/helper.ts"), [manual]);
    // Removed from the helper: the edge goes with the marker.
    write(&r, "test/helper.ts", &format!("{MOCK_HELPER}\n"));
    rebuild();
    assert!(!open_warm(&out), "a marker less: composed again");
    assert_eq!(check("a helper's manual mock removed"), 0);
    assert!(markers(&out, "test/helper.ts").is_empty());
    // Removed from the test.
    write(
        &r,
        "test/a.test.ts",
        &format!("import './helper';\n// moved\n{MOCK_TEST}test('more', () => {{}});\n"),
    );
    rebuild();
    assert!(!open_warm(&out), "a marker less: composed again");
    assert_eq!(check("a test's factory mock removed"), 0);
    assert!(markers(&out, "test/a.test.ts").is_empty());
}

/// A step of the reuse property: an edit that keeps a file's references (a line
/// above them, a line below them), a call of a mock API added to a file,
/// removed from it or moved in it, or any other edit.
#[derive(Clone, Debug)]
enum Step {
    Above(usize),
    Below(usize, u8),
    /// `(file, call, at the top)`: one more call of a mock API.
    MockAdd(usize, usize, bool),
    /// The file's first mock call goes.
    MockRemove(usize),
    /// The file's first mock call moves to the end of the file (from there: to
    /// the top).
    MockMove(usize),
    /// A line above everything in a file that may hold mock calls: every call
    /// moves down.
    MockShift(usize),
    Other(Edit),
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        3 => (0..FILES.len()).prop_map(Step::Above),
        3 => (0..FILES.len(), 0u8..4).prop_map(|(f, n)| Step::Below(f, n)),
        4 => (0..MOCK_FILES.len(), 0..MOCK_CALLS.len(), any::<bool>())
            .prop_map(|(f, c, top)| Step::MockAdd(f, c, top)),
        2 => (0..MOCK_FILES.len()).prop_map(Step::MockRemove),
        3 => (0..MOCK_FILES.len()).prop_map(Step::MockMove),
        2 => (0..MOCK_FILES.len()).prop_map(Step::MockShift),
        2 => edit().prop_map(Step::Other),
    ]
}

fn apply_step(r: &Path, s: &Step) {
    let rewrite = |rel: &str, edit: &dyn Fn(String) -> String| {
        let p = r.join(rel);
        // A file that is not there (yet, or any more) stays away: only content changes.
        if let Ok(text) = std::fs::read_to_string(&p)
            && !rel.ends_with(".json")
        {
            std::fs::write(&p, edit(text)).unwrap();
        }
    };
    let is_call = |l: &&str| l.starts_with("jest.");
    let join = |lines: &[&str]| -> String { lines.iter().map(|l| format!("{l}\n")).collect() };
    match s {
        Step::Above(f) => rewrite(FILES[*f], &|t| format!("// above\n{t}")),
        Step::Below(f, n) => rewrite(FILES[*f], &|t| format!("{t}export const added{n} = {n};\n")),
        Step::MockAdd(f, c, top) => rewrite(MOCK_FILES[*f], &|t| {
            let call = MOCK_CALLS[*c];
            if *top {
                format!("{call}\n{t}")
            } else {
                format!("{t}{call}\n")
            }
        }),
        Step::MockRemove(f) => rewrite(MOCK_FILES[*f], &|t| {
            let mut lines: Vec<&str> = t.lines().collect();
            if let Some(i) = lines.iter().position(is_call) {
                lines.remove(i);
            }
            join(&lines)
        }),
        Step::MockMove(f) => rewrite(MOCK_FILES[*f], &|t| {
            let mut lines: Vec<&str> = t.lines().collect();
            if let Some(i) = lines.iter().position(is_call) {
                let call = lines.remove(i);
                if i == lines.len() {
                    lines.insert(0, call);
                } else {
                    lines.push(call);
                }
            }
            join(&lines)
        }),
        Step::MockShift(f) => rewrite(MOCK_FILES[*f], &|t| format!("// above\n{t}")),
        Step::Other(e) => apply(r, e),
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    /// Incremental equals from scratch for the composed graph: after every step the graph
    /// opened from the refreshed store (its composed parts reused or not) exports what a
    /// fresh build exports, its composed and tables caches are byte-identical to a fresh
    /// build's, and so is its derived mock layer. The steps add, remove and move calls of
    /// the mock APIs in tests, helpers and setup files.
    #[test]
    fn a_reused_composition_equals_a_fresh_one(steps in proptest::collection::vec(step(), 1..7)) {
        let t = mock_fixture();
        let r = canonical(t.path());
        let out = r.join(".turbotrace");
        build(&opts(&r, &out, Externals::Opaque), &langs()).unwrap();
        open_warm(&out);
        for (i, s) in steps.iter().enumerate() {
            apply_step(&r, s);
            build(&opts(&r, &out, Externals::Opaque), &langs()).unwrap();
            let reused = open_warm(&out);
            if let Err(e) = same_as_fresh(&r, &out) {
                prop_assert!(false, "step {i} {s:?} (reused: {reused}): {e}");
            }
        }
    }
}
