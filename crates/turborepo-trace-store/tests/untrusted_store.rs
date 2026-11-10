//! A graph directory inside the repository (`.turbo/trace`) is
//! checkout-controlled, and it travels through caches: a commit can add a
//! `graph.json` and store files of its choosing. Names read from them stay
//! inside the store.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use turborepo_trace_core::Language;
use turborepo_trace_store::{
    Graph, GraphOptions, Manifest, build, gc,
    shard::{self, Pack},
};

fn langs() -> Vec<Arc<dyn Language>> {
    vec![Arc::new(turborepo_trace_lang_js::JsLanguage)]
}

fn write(root: &Path, rel: &str, contents: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, contents).unwrap();
}

/// A two-package repository at `<tmp>/repo`, its graph directory the default
/// `<repo>/.turbo/trace`.
fn repo() -> (tempfile::TempDir, PathBuf) {
    let t = tempfile::tempdir().unwrap();
    let r =
        turborepo_trace_core::fs::normalize(&std::fs::canonicalize(t.path()).unwrap()).join("repo");
    write(&r, ".gitignore", "node_modules\n.turbo\n");
    write(&r, "package.json", r#"{"name":"root","private":true}"#);
    write(&r, "pnpm-workspace.yaml", "packages:\n  - 'packages/*'\n");
    write(
        &r,
        "packages/a/package.json",
        r#"{"name":"a","version":"1.0.0"}"#,
    );
    write(&r, "packages/a/src/a.ts", "export const a = 1;\n");
    write(
        &r,
        "packages/b/package.json",
        r#"{"name":"b","version":"1.0.0","dependencies":{"a":"workspace:*"}}"#,
    );
    write(
        &r,
        "packages/b/src/b.ts",
        "import { a } from 'a';\nexport const b = a + 1;\n",
    );
    (t, r)
}

fn opts(root: &Path) -> GraphOptions {
    GraphOptions {
        root: root.to_path_buf(),
        threads: 2,
        ..Default::default()
    }
}

fn out(root: &Path) -> PathBuf {
    root.join(turborepo_trace_store::DEFAULT_GRAPH_PATH)
}

/// Rewrites `<out>/graph.json` as `edit` says, as a commit could.
fn tamper(out: &Path, edit: impl FnOnce(&mut serde_json::Value)) {
    let p = out.join("graph.json");
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    edit(&mut v);
    std::fs::write(&p, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
}

fn keys(m: &Manifest) -> Vec<String> {
    let mut k: Vec<String> = m.shards.iter().map(|s| s.key.clone()).collect();
    k.sort();
    k
}

#[test]
fn a_crafted_walked_file_list_name_deletes_nothing_outside_the_store() {
    let (t, r) = repo();
    let fresh = build(&opts(&r), &langs()).unwrap();
    let out = out(&r);
    let base = turborepo_trace_core::fs::normalize(&std::fs::canonicalize(t.path()).unwrap());
    // `<tmp>/victim` seen from `<tmp>/repo/.turbo/trace/store`, and by its absolute
    // path.
    let relative = base.join("victim");
    let absolute = base.join("victim-abs");
    std::fs::write(&relative, "keep").unwrap();
    std::fs::write(&absolute, "keep").unwrap();
    for name in [
        "../../../../victim".to_owned(),
        absolute.to_string_lossy().into_owned(),
    ] {
        tamper(&out, |v| {
            v["files"] = serde_json::Value::String(name.clone())
        });
        let rejected = (Manifest::read(&out).is_none(), Graph::open(&out).is_err());
        let again = build(&opts(&r), &langs()).unwrap();
        assert!(relative.exists() && absolute.exists(), "{name}: deleted");
        assert_eq!(rejected, (true, true), "{name}: not a graph");
        // The refresh writes a graph of its own, with the same shards.
        let m = Manifest::read(&out).unwrap();
        assert!(m.names_are_canonical());
        assert_eq!(keys(&m), keys(&fresh.manifest));
        assert_eq!(keys(&again.manifest), keys(&fresh.manifest));
    }
}

#[test]
fn a_crafted_shard_key_reaches_nothing_outside_the_store() {
    let (t, r) = repo();
    let fresh = build(&opts(&r), &langs()).unwrap();
    let out = out(&r);
    let store = out.join("store");
    let base = turborepo_trace_core::fs::normalize(&std::fs::canonicalize(t.path()).unwrap());
    // A file outside the store that is a valid shard: `gc` and compaction delete
    // the loose files of the shards they keep.
    let real = &fresh.manifest.shards[0].key;
    let bytes = Pack::open(&store).unwrap().get(real).unwrap().file_bytes();
    let victim = base.join("victim-shard");
    std::fs::write(&victim, &bytes).unwrap();
    let absolute = victim.to_string_lossy().into_owned();
    for key in ["../../../../victim-shard", absolute.as_str()] {
        tamper(&out, |v| {
            v["shards"][0]["key"] = serde_json::Value::String(key.to_owned());
        });
        let collected = gc(&out);
        assert!(victim.exists(), "{key}: deleted");
        assert!(collected.is_err(), "{key}");
        assert!(Manifest::read(&out).is_none(), "{key}: not a graph");
        assert!(Graph::open(&out).is_err(), "{key}");
        assert!(shard::load_from(&store, None, key).is_err(), "{key}");
        build(&opts(&r), &langs()).unwrap();
        assert!(victim.exists(), "{key}: deleted");
        assert_eq!(keys(&Manifest::read(&out).unwrap()), keys(&fresh.manifest));
    }
}
