//! End-to-end checks for the nft parity rules beyond plain `require`/`import`
//! (one fixture per root cause in `fixtures/js/resolve/nft-*`, traced with the
//! real [`JsLanguage`]): path values emitted as assets, require wrappers and
//! other loaders, and resolver conditions. See `fixtures/js/nft/README.md` for
//! the matching nft cases.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use turborepo_trace_core::{Trace, Tracer, TracerBuilder, UncertaintyKind, fs::normalize};
use turborepo_trace_lang_js::JsLanguage;

fn fixture(case: &str) -> PathBuf {
    normalize(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../turborepo-trace-testkit/fixtures/js/resolve")
            .join(case),
    )
}

struct Run {
    root: PathBuf,
    trace: Trace,
}

impl Run {
    fn new(builder: TracerBuilder, root: &Path, cwd: &Path, entry: &str) -> Run {
        let trace = builder
            .base(root)
            .cwd(cwd)
            .build()
            .expect("tracer")
            .trace(&[root.join(entry)])
            .expect("trace");
        Run {
            root: root.to_path_buf(),
            trace,
        }
    }

    fn files(&self) -> BTreeSet<String> {
        self.trace
            .files()
            .map(|p| {
                p.strip_prefix(&self.root)
                    .unwrap_or(p)
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    }

    /// `(kind, spec)` of every uncertainty.
    fn uncertainties(&self) -> Vec<(UncertaintyKind, String, String)> {
        self.trace
            .uncertainties()
            .into_iter()
            .map(|(_, u, e)| {
                (
                    u.kind,
                    e.map(|e| e.spec.to_string()).unwrap_or_default(),
                    u.detail.to_string(),
                )
            })
            .collect()
    }

    fn uncertainty(&self, spec: &str) -> Option<(UncertaintyKind, String)> {
        self.uncertainties()
            .into_iter()
            .find(|(_, s, _)| s == spec)
            .map(|(k, _, d)| (k, d))
    }
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn path_values_are_assets() {
    let root = fixture("nft-assets");
    let run = Run::new(
        Tracer::builder().language(JsLanguage),
        &root,
        &root,
        "main.js",
    );
    assert_eq!(
        run.files(),
        set(&[
            "assets/dict.json",
            "data/config.json",
            "img/logo.png",
            "main.js",
            "node_modules/wasm-pkg/lib/engine.wasm",
            "node_modules/wasm-pkg/lib/index.js",
            "node_modules/wasm-pkg/package.json",
            "notes.txt",
            "package.json",
            "schemas/a.json",
            "schemas/b.json",
            "views/page.html",
        ])
    );
    // Guesses carry records: the bundler namespace taken as `path` (and the cwd),
    // the directory expanded as a whole.
    let (k, d) = run
        .uncertainty("assets/dict.json")
        .expect("dict.json record");
    assert_eq!(k, UncertaintyKind::CwdDependent);
    assert!(d.contains("assumed to be `path.join`"), "{d}");
    let (k, d) = run.uncertainty("schemas").expect("schemas record");
    assert_eq!(k, UncertaintyKind::PartialSpecifier);
    assert!(d.contains("every file below it"), "{d}");
    // The missing path is a quiet probe; the anchors and the ancestor are never
    // emitted.
    assert!(run.uncertainty("missing.txt").is_none());
    assert!(
        run.uncertainties()
            .iter()
            .all(|(_, s, _)| s != ".." && s != ".")
    );
}

#[test]
fn loaders_beyond_require() {
    let root = fixture("nft-loaders");
    let run = Run::new(
        Tracer::builder().language(JsLanguage),
        &root,
        &root,
        "main.js",
    );
    assert_eq!(
        run.files(),
        set(&[
            "bundle.js",
            "hooks/loader.mjs",
            "lib-cov/index.js",
            "lib/index.js",
            "main.js",
            "node_modules/ext-dep/index.js",
            "node_modules/ext-dep/package.json",
            "node_modules/my-transport/index.js",
            "node_modules/my-transport/package.json",
            "node_modules/pino/index.js",
            "node_modules/pino/package.json",
            "node_modules/resolve-from/index.js",
            "node_modules/resolve-from/package.json",
            "package.json",
            "plugins/a.js",
            "plugins/b.js",
        ])
    );
    let kind = |spec: &str| run.uncertainty(spec).map(|(k, _)| k);
    // The wrapper is only called in the file and `target` is never reassigned: the
    // value-flow pass resolves both loads exactly.
    assert_eq!(kind("./plugins/a"), None);
    assert_eq!(kind("my-transport"), Some(UncertaintyKind::AliasedLoader));
    assert_eq!(kind("ext-dep"), Some(UncertaintyKind::AliasedLoader));
    assert_eq!(kind("./plugins/b.js"), None);
    // The bundled module is not required.
    assert!(run.uncertainty("./internal").is_none());
}

#[test]
fn module_sync_and_imports() {
    let root = fixture("nft-conditions");
    let run = Run::new(
        Tracer::builder().language(JsLanguage).target("linux/x64"),
        &root,
        &root,
        "main.js",
    );
    let files = run.files();
    for f in [
        "node_modules/sync-pkg/sync.js",
        "node_modules/sync-pkg/cjs.js",
        "internal-sync.js",
        "internal-default.js",
        "node_modules/browser-pkg/index.js",
        "node_modules/gyp-pkg/prebuilds/linuxglibc-x64/node.napi.node",
    ] {
        assert!(files.contains(f), "{f} missing: {files:?}");
    }
    // Not the `default` branch shadowed by `require`, not the browser file without
    // the `browser` condition, not the musl prebuild on glibc.
    for f in [
        "node_modules/sync-pkg/default.js",
        "node_modules/browser-pkg/browser.js",
        "node_modules/gyp-pkg/prebuilds/linuxmusl-x64/node.napi.node",
    ] {
        assert!(!files.contains(f), "{f} unexpected");
    }
}

#[test]
fn browser_field_with_browser_condition() {
    let root = fixture("nft-conditions");
    let run = Run::new(
        Tracer::builder()
            .language(JsLanguage)
            .conditions(["browser", "node", "import", "require", "default"]),
        &root,
        &root,
        "main.js",
    );
    let files = run.files();
    assert!(files.contains("node_modules/browser-pkg/browser.js"));
    assert!(files.contains("node_modules/browser-pkg/index.js"));
}

#[test]
fn readdir_of_own_directory_that_is_the_cwd() {
    let root = fixture("nft-conditions");
    let run = Run::new(
        Tracer::builder().language(JsLanguage),
        &root,
        &root.join("listing"),
        "listing/index.js",
    );
    assert!(
        run.files().contains("listing/sub/file.txt"),
        "{:?}",
        run.files()
    );
}
