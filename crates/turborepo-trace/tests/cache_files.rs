//! Every file a command keeps in the graph's caches is one the eviction knows
//! ([`turborepo_trace_store::CACHE_FILES`]): a cache added without a name there
//! is never aged out and does not count toward `cacheMaxSize`.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use turborepo_trace::commands::{AffectedCmd, GraphLoc, affected};
use turborepo_trace_store::cache_file;

fn write(root: &Path, rel: &str, contents: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, contents).unwrap();
}

/// A repository whose build and query write the caches of the walk's hashes and
/// setup files, composition, the derived layers (environment, manual mocks,
/// cross-file binding) and the cross-file facts: `table.ts` loads a module
/// through a key its caller passes, and the tests of `packages/x` reach it.
fn repo() -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    let r = t.path();
    write(r, ".gitignore", "node_modules\n.turbo\n");
    write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(r, "turbo.json", r#"{"tasks":{"test":{"outputs":[]}}}"#);
    write(
        r,
        "src/table.ts",
        "const BY: Record<string, string> = { a: './a', b: './b' };\nexport function load(names: \
         string[]) {\n\x20 const out: unknown[] = [];\n\x20 for (const n of names) {\n\x20   \
         out.push(require(BY[n]));\n\x20 }\n\x20 return out;\n}\n",
    );
    write(r, "src/a.ts", "export const a = 1;\n");
    write(r, "src/b.ts", "export const b = 2;\n");
    write(
        r,
        "src/setup.ts",
        "export const region = process.env.REGION ?? 'eu';\n",
    );
    write(
        r,
        "src/mock.ts",
        "jest.mock('./sends', () => ({ send: () => 'mock' }));\n",
    );
    write(
        r,
        "src/sends.ts",
        "export function send() { return 'real'; }\n",
    );
    write(
        r,
        "packages/x/package.json",
        r#"{"name":"x","version":"1.0.0","scripts":{"test":"jest"}}"#,
    );
    write(
        r,
        "packages/x/jest.config.js",
        "module.exports = { testMatch: ['<rootDir>/test/**/*.test.ts'] };\n",
    );
    write(
        r,
        "packages/x/test/a.test.ts",
        "import { load } from '../../../src/table';\nimport { region } from \
         '../../../src/setup';\nimport '../../../src/mock';\ntest('a', () => { load(['a']); \
         region; });\n",
    );
    write(
        r,
        "packages/x/test/b.test.ts",
        "import { load } from '../../../src/table';\ntest('b', () => { load(['b']); });\n",
    );
    t
}

/// The names in the cache directory of `out`.
fn cache_names(out: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(turborepo_trace_store::cache_dir(out))
        .map(|d| {
            d.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// `affected` writes every cache of a build and of a query, and the eviction
/// knows all of them. The walk, the observation cache and the names layer's
/// data are not among them: a fresh fixture's walk is not trusted (its files
/// are younger than the walk cache's racy window) and it has no computed read
/// the names layer can name, so those three are written by other runs.
#[test]
fn every_cache_a_command_writes_is_one_eviction_knows() {
    let t = repo();
    let root = std::fs::canonicalize(t.path()).unwrap();
    let out: PathBuf = root.join(turborepo_trace_store::DEFAULT_GRAPH_PATH);
    let cmd = AffectedCmd {
        loc: GraphLoc {
            root: Some(root.clone()),
            graph: None,
            ..Default::default()
        },
        tests: vec!["jest".to_owned()],
        changed: vec!["src/a.ts".to_owned()],
        ..Default::default()
    };
    affected(cmd, &mut std::io::sink()).unwrap();
    turborepo_trace_store::wait_for_cache_writes();

    let names = cache_names(&out);
    // The run wrote these: without them the check below would pass for the wrong
    // reason.
    for name in [
        cache_file::HASHES,
        cache_file::MANIFESTS,
        cache_file::COMPOSED,
        cache_file::COMPOSED_FOR,
        cache_file::TABLES,
        cache_file::DERIVED_ENV,
        cache_file::DERIVED_MOCKS,
        cache_file::DERIVED_BOUND,
        cache_file::XFILE_FACTS,
        cache_file::XFILE_TRACE,
    ] {
        assert!(
            names.iter().any(|n| n == name),
            "{name} was not written: {names:?}"
        );
    }
    let unknown: Vec<&String> = names
        .iter()
        // The note naming the graph directory a per-machine cache belongs to.
        .filter(|name| name.as_str() != "graph-dir")
        .filter(|name| !turborepo_trace_store::CACHE_FILES.contains(&name.as_str()))
        .collect();
    assert!(
        unknown.is_empty(),
        "caches eviction does not know: {unknown:?} (it knows {:?})",
        turborepo_trace_store::CACHE_FILES
    );
}
