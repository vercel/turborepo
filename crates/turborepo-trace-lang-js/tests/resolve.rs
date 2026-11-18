//! Resolver tests for `fixtures/js/resolve/`.
//!
//! `resolver_*` tests call [`JsResolver`] directly with hand-built [`RawRef`]s,
//! so they do not depend on the extractor. `snapshot_*` tests trace the
//! fixtures end to end with [`JsLanguage`] and snapshot the JSON output.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use turborepo_trace_core::{
    DirCache, Externals, FileId, PathInterner, ProjectCx, RawRef, RefFlags, RefKind, ResolveCx,
    Resolver, Span, Specifier, Target, TraceOptions, Tracer, UncertaintyKind, fs::normalize,
};
use turborepo_trace_lang_js::{
    JsLanguage,
    resolve::{JsResolveOptions, JsResolver},
};
use turborepo_trace_testkit::fixture_snapshot;

fn fixtures() -> PathBuf {
    normalize(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../turborepo-trace-testkit/fixtures/js/resolve"),
    )
}

struct Env {
    root: PathBuf,
    fs: Arc<DirCache>,
    interner: PathInterner,
    options: TraceOptions,
    resolver: JsResolver,
}

/// What one resolution produced, with paths relative to the fixture.
#[derive(Debug, PartialEq, Eq)]
struct Out {
    targets: Vec<String>,
    manifests: Vec<String>,
    uncertainty: Option<UncertaintyKind>,
    detail: String,
}

impl Env {
    fn new(case: &str) -> Self {
        Self::with(case, TraceOptions::default(), JsResolveOptions::default())
    }

    fn with(case: &str, options: TraceOptions, js: JsResolveOptions) -> Self {
        Self::with_fs(case, options, js, DirCache::new())
    }

    fn with_fs(case: &str, options: TraceOptions, js: JsResolveOptions, fs: DirCache) -> Self {
        Self::at(fixtures().join(case), options, js, fs)
    }

    /// An environment for the tree at `root`.
    fn at(root: PathBuf, mut options: TraceOptions, js: JsResolveOptions, fs: DirCache) -> Self {
        options.base = root.clone();
        options.cwd = root.clone();
        let fs = Arc::new(fs);
        let resolver = JsResolver::new(
            &ProjectCx {
                options: &options,
                fs: &fs,
            },
            &js,
        );
        Env {
            root,
            fs,
            interner: PathInterner::new(),
            options,
            resolver,
        }
    }

    fn rel(&self, id: FileId) -> String {
        let p = self.interner.path(id);
        p.strip_prefix(&self.root)
            .map(|r| r.to_string_lossy().into_owned())
            .unwrap_or_else(|_| p.to_string_lossy().into_owned())
    }

    fn resolve(&self, from: &str, kind: RefKind, spec: &str) -> Out {
        self.resolve_ref(
            from,
            RawRef::new(kind, Specifier::literal(spec), Span::new(0, 1)),
        )
    }

    fn resolve_ref(&self, from: &str, r: RawRef<'_>) -> Out {
        let from_path = self.root.join(from);
        let cx = ResolveCx {
            from_path: &from_path,
            from_dir: from_path.parent().unwrap(),
            options: &self.options,
            fs: &self.fs,
            interner: &self.interner,
        };
        let id = self.interner.intern(&from_path);
        let res = self.resolver.resolve(&r, id, &cx);
        Out {
            targets: res
                .targets
                .iter()
                .map(|t| match t {
                    Target::File(id) => self.rel(*id),
                    Target::Dir(id) => format!("dir:{}", self.rel(*id)),
                    Target::Builtin(n) => format!("builtin:{n}"),
                    Target::External(n) => format!("external:{n}"),
                    Target::Unresolved => "unresolved".into(),
                    Target::Glob { pattern, .. } => format!("glob:{pattern}"),
                })
                .collect(),
            manifests: res.manifests.iter().map(|m| self.rel(*m)).collect(),
            uncertainty: res.uncertainty.as_ref().map(|u| u.kind),
            detail: res
                .uncertainty
                .map(|u| u.detail.to_string())
                .unwrap_or_default(),
        }
    }

    /// Uncertainty details of the file-level edges of `from`.
    fn file_uncertainties(&self, from: &str) -> Vec<(UncertaintyKind, String)> {
        let from_path = self.root.join(from);
        let cx = ResolveCx {
            from_path: &from_path,
            from_dir: from_path.parent().unwrap(),
            options: &self.options,
            fs: &self.fs,
            interner: &self.interner,
        };
        let id = self.interner.intern(&from_path);
        self.resolver
            .file_edges(id, &cx)
            .into_iter()
            .filter_map(|x| x.uncertainty.map(|u| (u.kind, u.detail.to_string())))
            .collect()
    }

    fn targets(&self, from: &str, kind: RefKind, spec: &str) -> Vec<String> {
        let out = self.resolve(from, kind, spec);
        assert_eq!(out.uncertainty, None, "{spec}: {out:?}");
        out.targets
    }
}

use RefKind::{DynamicImport, Import, ReExport, Require};

#[test]
fn resolver_relative_index_and_extensions() {
    let e = Env::new("basic");
    assert_eq!(e.targets("main.js", Require, "./src/rel"), ["src/rel.js"]);
    assert_eq!(
        e.targets("main.js", Require, "./src/rel.js"),
        ["src/rel.js"]
    );
    assert_eq!(
        e.targets("main.js", Require, "./src/dir"),
        ["src/dir/index.js"]
    );
    assert_eq!(e.targets("main.js", Require, "./data.json"), ["data.json"]);
    assert_eq!(
        e.targets("src/dir/index.js", Require, "../rel"),
        ["src/rel.js"]
    );
    let abs = e.root.join("src/rel").to_string_lossy().into_owned();
    assert_eq!(e.targets("main.js", Require, &abs), ["src/rel.js"]);
}

#[test]
fn resolver_package_main_and_main_fields() {
    let e = Env::new("basic");
    assert_eq!(
        e.targets("main.js", Require, "pkg-main"),
        ["node_modules/pkg-main/lib/entry.js"]
    );
    // Node ignores `module`.
    assert_eq!(
        e.targets("esm.mjs", Import, "pkg-module"),
        ["node_modules/pkg-module/dist/cjs.js"]
    );
    // Configurable main fields.
    let e = Env::with(
        "basic",
        TraceOptions::default(),
        JsResolveOptions {
            main_fields: vec!["module".into(), "main".into()],
            ..Default::default()
        },
    );
    assert_eq!(
        e.targets("esm.mjs", Import, "pkg-module"),
        ["node_modules/pkg-module/dist/esm.js"]
    );
}

#[test]
fn resolver_exports_conditions_follow_the_module_system() {
    let e = Env::new("basic");
    // The `exports` target first, then the legacy `main` (nft without
    // `exportsOnly`).
    assert_eq!(
        e.targets("main.js", Require, "pkg-exports"),
        [
            "node_modules/pkg-exports/cjs/index.cjs",
            "node_modules/pkg-exports/legacy.js"
        ]
    );
    for kind in [Import, DynamicImport, ReExport] {
        assert_eq!(
            e.targets("esm.mjs", kind, "pkg-exports"),
            [
                "node_modules/pkg-exports/esm/index.mjs",
                "node_modules/pkg-exports/legacy.js"
            ],
            "{kind:?}"
        );
    }
    // Subpath patterns.
    assert_eq!(
        e.targets("main.js", Require, "pkg-exports/features/a"),
        ["node_modules/pkg-exports/src/features/a.js"]
    );
    // Not exported.
    let out = e.resolve("main.js", Require, "pkg-exports/legacy.js");
    assert_eq!(out.uncertainty, Some(UncertaintyKind::UnresolvedSpecifier));
    // `exports` only, no `main`: no legacy target.
    assert_eq!(
        e.targets("main.js", Require, "pkg-exports-only"),
        ["node_modules/pkg-exports-only/only.js"]
    );

    // exportsOnly: no legacy target.
    let e = Env::with(
        "basic",
        TraceOptions::default(),
        JsResolveOptions {
            exports_only: true,
            ..Default::default()
        },
    );
    assert_eq!(
        e.targets("main.js", Require, "pkg-exports"),
        ["node_modules/pkg-exports/cjs/index.cjs"]
    );
}

#[test]
fn resolver_custom_conditions() {
    // `--conditions` replaces the default set; with only `require` the ESM import
    // has no matching condition.
    let opts = TraceOptions {
        conditions: vec!["require".into()],
        ..Default::default()
    };
    let e = Env::with(
        "basic",
        opts,
        JsResolveOptions {
            exports_only: true,
            ..Default::default()
        },
    );
    assert_eq!(
        e.targets("main.js", Require, "pkg-exports"),
        ["node_modules/pkg-exports/cjs/index.cjs"]
    );
    let out = e.resolve("esm.mjs", Import, "pkg-exports");
    assert_eq!(out.uncertainty, Some(UncertaintyKind::UnresolvedSpecifier));
}

#[test]
fn resolver_imports_self_reference_and_scoped() {
    let e = Env::new("basic");
    assert_eq!(
        e.targets("main.js", Require, "#internal/util"),
        ["src/internal/util.js"]
    );
    assert_eq!(
        e.targets("main.js", Require, "app/feature"),
        ["src/feature.js"]
    );
    assert_eq!(e.targets("esm.mjs", Import, "app"), ["main.js"]);
    assert_eq!(
        e.targets("main.js", Require, "@scope/pkg"),
        ["node_modules/@scope/pkg/index.js"]
    );
    // Nested resolution from inside node_modules.
    assert_eq!(
        e.targets("node_modules/@scope/pkg/index.js", Require, "pkg-main"),
        ["node_modules/pkg-main/lib/entry.js"]
    );
}

#[test]
fn resolver_builtins() {
    let e = Env::new("basic");
    for (spec, name) in [
        ("fs", "node:fs"),
        ("node:fs", "node:fs"),
        ("fs/promises", "node:fs/promises"),
        ("node:fs/promises", "node:fs/promises"),
        ("node:test", "node:test"),
        ("path", "node:path"),
    ] {
        assert_eq!(
            e.targets("main.js", Require, spec),
            [format!("builtin:{name}")],
            "{spec}"
        );
    }
}

#[test]
fn resolver_manifests() {
    let e = Env::new("basic");
    // The referencing file's scope first, then the package root, then the target's
    // scope.
    let out = e.resolve("esm.mjs", Import, "pkg-exports");
    assert_eq!(
        out.manifests,
        [
            "package.json",
            "node_modules/pkg-exports/package.json",
            "node_modules/pkg-exports/esm/package.json",
        ]
    );
    let out = e.resolve("main.js", Require, "./src/rel");
    assert_eq!(out.manifests, ["package.json"]);
    let out = e.resolve("main.js", Require, "fs");
    assert_eq!(out.manifests, ["package.json"]);
    // Unresolved references still record the referencing file's scope.
    let out = e.resolve("main.js", Require, "does-not-exist");
    assert_eq!(out.manifests, ["package.json"]);
}

#[test]
fn resolver_failures_keep_specifier_and_file() {
    let e = Env::new("basic");
    for spec in ["does-not-exist", "./nope", "@scope/missing/sub"] {
        let out = e.resolve("main.js", Require, spec);
        assert_eq!(out.targets, ["unresolved"]);
        assert_eq!(out.uncertainty, Some(UncertaintyKind::UnresolvedSpecifier));
        assert!(out.detail.contains(spec), "{}", out.detail);
        assert!(out.detail.contains("main.js"), "{}", out.detail);
    }
    let out = e.resolve("main.js", Require, "./src/REL.js");
    assert_eq!(out.targets, ["src/rel.js"]);
    assert_eq!(out.uncertainty, Some(UncertaintyKind::CaseMismatch));
}

/// P10: a load the code tolerates failing (inside a `try` whose `catch`
/// swallows the error, flagged `OPTIONAL` by the extractor) keeps its
/// unresolved edge (probes, lockfile fallback) but is no uncertainty. Computed
/// optional specifiers stay uncertain.
#[test]
fn optional_failures_are_not_uncertain() {
    let e = Env::new("basic");
    for spec in ["does-not-exist", "./nope", "@scope/missing/sub"] {
        let r = RawRef::new(Require, Specifier::literal(spec), Span::new(0, 1))
            .with_flags(RefFlags::OPTIONAL);
        let out = e.resolve_ref("main.js", r);
        assert_eq!(out.targets, ["unresolved"], "{spec}");
        assert_eq!(out.uncertainty, None, "{spec}");
    }
    // A resolvable optional load resolves as usual.
    let r = RawRef::new(Require, Specifier::literal("./src/rel"), Span::new(0, 1))
        .with_flags(RefFlags::OPTIONAL);
    assert_eq!(e.resolve_ref("main.js", r).targets, ["src/rel.js"]);
}

#[test]
fn resolver_typescript() {
    let e = Env::new("ts");
    assert_eq!(e.targets("src/main.ts", Import, "./a.js"), ["src/a.ts"]);
    assert_eq!(e.targets("src/main.ts", Import, "@lib/b"), ["src/lib/b.ts"]);
    assert_eq!(e.targets("src/main.ts", Import, "./side"), ["src/side.tsx"]);
    assert_eq!(e.targets("src/main.ts", Import, "./c.mjs"), ["src/c.mts"]);
    assert_eq!(
        e.targets("src/main.ts", Import, "./plain.js"),
        ["src/plain.js"]
    );
    // JS files fall back from a missing `.js` to the `.ts` source.
    assert_eq!(e.targets("src/plain.js", Import, "./a.js"), ["src/a.ts"]);
    // tsconfig `paths` apply to JS files too, as in bundlers: `plain.js` is not
    // covered (no `allowJs`), so the package's nearest config applies.
    assert_eq!(
        e.targets("src/plain.js", Import, "@lib/b"),
        ["src/lib/b.ts"]
    );
    // With TypeScript's ownership rules only, it is not.
    let strict = Env::with(
        "ts",
        TraceOptions::default(),
        JsResolveOptions {
            tsconfig_fallback: false,
            ..Default::default()
        },
    );
    let out = strict.resolve("src/plain.js", Import, "@lib/b");
    assert_eq!(out.uncertainty, Some(UncertaintyKind::UnresolvedSpecifier));
    assert_eq!(
        strict.targets("src/main.ts", Import, "@lib/b"),
        ["src/lib/b.ts"]
    );
    // Declaration-only targets are excluded by default...
    let out = e.resolve("src/main.ts", Import, "./decl");
    assert_eq!(out.uncertainty, Some(UncertaintyKind::UnresolvedSpecifier));
    assert!(out.detail.contains("decl.d.ts"), "{}", out.detail);
    let out = e.resolve("src/main.ts", Import, "./decl.d.ts");
    assert_eq!(out.uncertainty, Some(UncertaintyKind::UnresolvedSpecifier));
    // ...and included with --include-types.
    let e = Env::with(
        "ts",
        TraceOptions {
            include_types: true,
            ..Default::default()
        },
        JsResolveOptions::default(),
    );
    assert_eq!(
        e.targets("src/main.ts", Import, "./decl"),
        ["src/decl.d.ts"]
    );
}

#[test]
fn resolver_tsconfig_paths_for_js_files() {
    let e = Env::new("tsconfig-js");
    // Covered by `include` with `allowJs` (TypeScript's rules).
    for from in ["src/page.js", "src/esm.mjs", "src/main.ts"] {
        assert_eq!(
            e.targets(from, Import, "#/lib/util"),
            ["lib/util.ts"],
            "{from}"
        );
    }
    // `.jsx` is not in `include`: the package's config applies anyway (Next.js).
    assert_eq!(
        e.targets("src/widget.jsx", Import, "#/lib/util"),
        ["lib/util.ts"]
    );
    // A JS-only app with `jsconfig.json`, from CommonJS.
    assert_eq!(
        e.targets("jsapp/src/index.js", Require, "@/components/button"),
        ["jsapp/components/button.js"]
    );
    // TypeScript's rules only: `.jsx` is not covered and gets no `paths`.
    let strict = Env::with(
        "tsconfig-js",
        TraceOptions::default(),
        JsResolveOptions {
            tsconfig_fallback: false,
            ..Default::default()
        },
    );
    assert_eq!(
        strict.targets("src/page.js", Import, "#/lib/util"),
        ["lib/util.ts"]
    );
    let out = strict.resolve("src/widget.jsx", Import, "#/lib/util");
    assert_eq!(out.uncertainty, Some(UncertaintyKind::UnresolvedSpecifier));
    // Without tsconfig support, no `paths` at all.
    let off = Env::with(
        "tsconfig-js",
        TraceOptions::default(),
        JsResolveOptions {
            tsconfig: false,
            ..Default::default()
        },
    );
    let out = off.resolve("src/page.js", Import, "#/lib/util");
    assert_eq!(out.uncertainty, Some(UncertaintyKind::UnresolvedSpecifier));
}

#[test]
fn resolver_tsconfig_broken_references_degrade() {
    let e = Env::new("tsconfig-broken");
    // The root config's `paths` and plain relative imports keep working.
    assert_eq!(e.targets("src/main.ts", Import, "@/util"), ["src/util.ts"]);
    assert_eq!(e.targets("src/main.ts", Import, "./rel"), ["src/rel.ts"]);
    // A reference that loads governs its files.
    assert_eq!(
        e.targets("pkgs/good/src/main.ts", Import, "#good/x"),
        ["pkgs/good/src/x.ts"]
    );
    // One uncertainty for the config, naming every skipped reference, reported
    // once.
    let u = e.file_uncertainties("src/main.ts");
    assert_eq!(u.len(), 1, "{u:?}");
    assert_eq!(u[0].0, UncertaintyKind::ParseIncomplete);
    let d = &u[0].1;
    assert!(d.contains("tsconfig-broken/tsconfig.json"), "{d}");
    for skipped in ["pkgs/missing", "pkgs/bad-json"] {
        assert!(
            d.contains(&format!(
                "skipped project reference {}",
                e.root.join(skipped).display()
            )),
            "{skipped}: {d}"
        );
    }
    // `pkgs/bad` extends a package that isn't installed: it loads without that base
    // (a third-party base can't define first-party path mappings), so it is not
    // skipped.
    assert!(!d.contains("@not-installed/tsconfig/base.json"), "{d}");
    assert!(!d.contains("pkgs/bad:"), "{d}");
    assert!(!d.contains("pkgs/good"), "{d}");
    assert_eq!(e.file_uncertainties("src/util.ts"), []);
    assert_eq!(e.file_uncertainties("pkgs/good/src/main.ts"), []);
}

/// A tsconfig whose `extends` names packages that aren't installed (a checkout
/// without `node_modules`) still applies: its own `paths` and the `baseUrl` of
/// a base that exists. No uncertainty: a third-party base can't define
/// first-party path mappings.
#[test]
fn resolver_tsconfig_with_uninstalled_bases() {
    let e = Env::new("tsconfig-missing-base");
    assert_eq!(e.targets("src/main.ts", Import, "@/util"), ["src/util.ts"]);
    assert_eq!(
        e.targets("src/main.ts", Import, "src/deep"),
        ["src/deep.ts"]
    );
    assert_eq!(e.file_uncertainties("src/main.ts"), []);
}

#[test]
fn resolver_records_config_discovery_for_js_files() {
    let e = Env::with_fs(
        "tsconfig-broken",
        TraceOptions::default(),
        JsResolveOptions::default(),
        DirCache::with_recording(),
    );
    let frame = turborepo_trace_core::record::start();
    let u = e.file_uncertainties("src/main.ts");
    let obs = frame.finish();
    // In graph mode the load failures are reported on every file that consults the
    // config.
    assert_eq!(u.len(), 1);
    assert_eq!(e.file_uncertainties("src/util.ts").len(), 1);
    let rel = |p: &PathBuf| p.strip_prefix(&e.root).map(|r| r.display().to_string());
    let reads: Vec<_> = obs.reads.iter().filter_map(|p| rel(p).ok()).collect();
    let missing: Vec<_> = obs.missing.iter().filter_map(|p| rel(p).ok()).collect();
    for want in [
        "tsconfig.json",
        "pkgs/bad-json/tsconfig.json",
        "pkgs/bad-json/base.json",
    ] {
        assert!(reads.iter().any(|r| r == want), "{want} in {reads:?}");
    }
    // P13: a reference that loads but doesn't govern the file is no content input.
    // That includes `pkgs/bad`, which loads without its uninstalled base.
    // It is a key observation: whether it loads and what it includes decide the
    // file's records, so a change of it makes them stale, and no query selects
    // through it.
    let key_reads: Vec<_> = obs
        .key
        .iter()
        .flat_map(|set| &set.reads)
        .filter_map(|p| rel(p).ok())
        .collect();
    for loaded in ["pkgs/good/tsconfig.json", "pkgs/bad/tsconfig.json"] {
        assert!(!reads.iter().any(|r| r == loaded), "{loaded} in {reads:?}");
        assert!(
            key_reads.iter().any(|r| r == loaded),
            "{loaded} in {key_reads:?}"
        );
    }
    assert!(obs.key.iter().all(|set| set.key.is_empty()));
    for want in [
        "src/tsconfig.json",
        "src/jsconfig.json",
        "pkgs/missing",
        "pkgs/missing.json",
    ] {
        assert!(missing.iter().any(|r| r == want), "{want} in {missing:?}");
    }

    // JS files record their chain too.
    let e = Env::with_fs(
        "tsconfig-js",
        TraceOptions::default(),
        JsResolveOptions::default(),
        DirCache::with_recording(),
    );
    let frame = turborepo_trace_core::record::start();
    assert_eq!(e.file_uncertainties("jsapp/src/index.js"), []);
    let obs = frame.finish();
    let reads: Vec<_> = obs.reads.iter().filter_map(|p| rel2(&e.root, p)).collect();
    assert!(
        reads.iter().any(|r| r == "jsapp/jsconfig.json"),
        "{reads:?}"
    );
    // P13: `jsapp/jsconfig.json` covers the file, so discovery never looks above
    // it.
    assert!(!reads.iter().any(|r| r == "tsconfig.json"), "{reads:?}");
}

/// What a file's config discovery records, relative to `root`: the files it
/// read (the manifests' views, the links it went through and the key
/// observations included), and the places it looked at and found nothing.
fn config_record(root: &Path, file: &str) -> (Vec<String>, Vec<String>) {
    let e = Env::at(
        root.to_path_buf(),
        TraceOptions::default(),
        JsResolveOptions::default(),
        DirCache::with_recording(),
    );
    let frame = turborepo_trace_core::record::start();
    e.file_uncertainties(file);
    let obs = frame.finish();
    // A path through a package directory (`node_modules/<name>`, a link to a
    // workspace package) is recorded as it was asked: the store records it
    // where the directory leads. Here both.
    let through = |p: &PathBuf| -> Vec<PathBuf> {
        let mut out = vec![p.clone()];
        if let Some(there) = p.ancestors().find(|a| a.exists())
            && let (Ok(real), Ok(rest)) = (std::fs::canonicalize(there), p.strip_prefix(there))
        {
            out.push(real.join(rest));
        }
        out
    };
    let (mut read, mut missing) = (Vec::new(), Vec::new());
    for o in std::iter::once(&obs).chain(obs.key.iter().map(|k| &**k)) {
        // (A manifest: through the resolution view, or the view of a config loader.)
        let paths = (o.reads.iter().chain(&o.view_reads))
            .chain(&o.loader_reads)
            .chain(o.path_reads.iter().chain(&o.links));
        read.extend(paths.flat_map(through).filter_map(|p| rel2(root, &p)));
        missing.extend(
            o.missing
                .iter()
                .flat_map(through)
                .filter_map(|p| rel2(root, &p)),
        );
    }
    read.sort();
    read.dedup();
    missing.sort();
    missing.dedup();
    (read, missing)
}

/// The base config that two configs extend is in the record of the files of
/// both: each load is observed by itself, whichever came first (a loader that
/// answered the second load from what the first left behind would read nothing
/// of the base again).
#[test]
fn resolver_records_a_shared_base_for_every_config_that_extends_it() {
    let t = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(t.path()).unwrap();
    let write = |rel: &str, text: &str| {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    };
    write("package.json", r#"{"name":"root"}"#);
    write("tsconfig.base.json", "{}");
    for p in ["a", "b"] {
        write(
            &format!("{p}/package.json"),
            &format!("{{\"name\":\"{p}\"}}"),
        );
        write(
            &format!("{p}/tsconfig.json"),
            r#"{"extends":"../tsconfig.base"}"#,
        );
        write(&format!("{p}/src/main.ts"), "");
    }
    let e = Env::at(
        root.clone(),
        TraceOptions::default(),
        JsResolveOptions::default(),
        DirCache::with_recording(),
    );
    for p in ["a", "b", "a"] {
        let frame = turborepo_trace_core::record::start();
        e.file_uncertainties(&format!("{p}/src/main.ts"));
        let obs = frame.finish();
        let reads: Vec<String> = obs.reads.iter().filter_map(|r| rel2(&root, r)).collect();
        assert!(
            reads.iter().any(|r| r == "tsconfig.base.json"),
            "{p}: {reads:?}"
        );
        assert!(
            obs.missing.contains(&root.join("tsconfig.base")),
            "{p}: the name as written is looked for first: {:?}",
            obs.missing
        );
    }
}

/// The record of a config's load against the loader itself, on layouts of every
/// kind of `extends` and reference: whatever changes what the file's alias
/// resolves to (or whether its config loads) is in the file's record. A file
/// whose contents change or that goes is among the files read; a link that goes
/// is recorded; a file that appears is at a place that was looked at, or below
/// one. Nothing here knows how the loader finds a file: the layouts are only
/// there to make it take every way it has.
#[test]
fn resolver_records_everything_a_config_load_depends_on() {
    const V1: &str = r#"{"compilerOptions":{"paths":{"@lib/*":["${configDir}/v1/*"]}}}"#;
    const V2: &str = r#"{"compilerOptions":{"paths":{"@lib/*":["${configDir}/v2/*"]}}}"#;
    const FILE: &str = "app/src/main.ts";
    // (name, the app's manifest, the app's config, other files, links as (link,
    // target)).
    type Layout = (
        &'static str,
        &'static str,
        &'static str,
        &'static [(&'static str, &'static str)],
        &'static [(&'static str, &'static str)],
    );
    const APP: &str = r#"{"name":"app"}"#;
    const TOOLING: (&str, &str) = ("app/node_modules/@acme/cfg", "../../../tooling/cfg");
    let layouts: &[Layout] = &[
        (
            "a file",
            APP,
            r#"{"extends":"./base.json"}"#,
            &[("app/base.json", V1)],
            &[],
        ),
        (
            "a dotted name without .json",
            APP,
            r#"{"extends":"./lib.base"}"#,
            &[("app/lib.base.json", V1), ("app/lib.json", V2)],
            &[],
        ),
        (
            "a dotted name above, next to a tsconfig.json",
            APP,
            r#"{"extends":"../tsconfig.base"}"#,
            &[("tsconfig.base.json", V1), ("tsconfig.json", "{}")],
            &[],
        ),
        (
            "a directory",
            APP,
            r#"{"extends":"./configs"}"#,
            &[("app/configs/tsconfig.json", V1)],
            &[],
        ),
        (
            "a chain",
            APP,
            r#"{"extends":"./mid"}"#,
            &[
                ("app/mid.json", r#"{"extends":"../shared/base"}"#),
                ("shared/base.json", V1),
            ],
            &[],
        ),
        (
            "an array",
            APP,
            r#"{"extends":["./one.base","./two.json"]}"#,
            &[("app/one.base.json", V1), ("app/two.json", "{}")],
            &[],
        ),
        (
            "a package's exports",
            APP,
            r#"{"extends":"@acme/cfg/lib"}"#,
            &[
                (
                    "tooling/cfg/package.json",
                    r#"{"name":"@acme/cfg","exports":{"./lib":"./configs/lib.json"}}"#,
                ),
                ("tooling/cfg/configs/lib.json", V1),
            ],
            &[TOOLING],
        ),
        (
            "a file of a package",
            APP,
            r#"{"extends":"@acme/cfg/lib.json"}"#,
            &[
                ("tooling/cfg/package.json", r#"{"name":"@acme/cfg"}"#),
                ("tooling/cfg/lib.json", V1),
            ],
            &[TOOLING],
        ),
        (
            "a package",
            APP,
            r#"{"extends":"@acme/cfg"}"#,
            &[
                ("tooling/cfg/package.json", r#"{"name":"@acme/cfg"}"#),
                ("tooling/cfg/tsconfig.json", V1),
            ],
            &[TOOLING],
        ),
        (
            "a package a level above",
            APP,
            r#"{"extends":"@acme/cfg/lib.json"}"#,
            &[
                ("tooling/cfg/package.json", r#"{"name":"@acme/cfg"}"#),
                ("tooling/cfg/lib.json", V1),
            ],
            &[("node_modules/@acme/cfg", "../../tooling/cfg")],
        ),
        (
            "a subpath import",
            r##"{"name":"app","imports":{"#cfg":"./configs/base.json"}}"##,
            r##"{"extends":"#cfg"}"##,
            &[("app/configs/base.json", V1)],
            &[],
        ),
        (
            "the package's own name",
            r#"{"name":"app","exports":{"./base":"./configs/base.json"}}"#,
            r#"{"extends":"app/base"}"#,
            &[("app/configs/base.json", V1)],
            &[],
        ),
        (
            "a link",
            APP,
            r#"{"extends":"./link.json"}"#,
            &[("shared/base.json", V1)],
            &[("app/link.json", "../shared/base.json")],
        ),
        (
            "a linked directory",
            APP,
            r#"{"extends":"./linked/base.json"}"#,
            &[("shared/base.json", V1)],
            &[("app/linked", "../shared")],
        ),
        (
            "a reference without .json",
            APP,
            r#"{"references":[{"path":"./proj"}]}"#,
            &[
                (
                    "app/proj.json",
                    r#"{"extends":"./lib.base","include":["src"]}"#,
                ),
                ("app/lib.base.json", V1),
            ],
            &[],
        ),
        (
            "a reference that is a link",
            APP,
            r#"{"references":[{"path":"./proj.json"}]}"#,
            &[(
                "shared/proj.json",
                r#"{"compilerOptions":{"paths":{"@lib/*":["${configDir}/v1/*"]}},"include":["${configDir}/src"]}"#,
            )],
            &[("app/proj.json", "../shared/proj.json")],
        ),
        // A directory at the name, without a config: the loader looks inside it and
        // never at the name with `.json`, and the config (or the project) does not load.
        // An answer is an observation, whichever answer it is: the directory that goes
        // is found by what was looked for inside it.
        (
            "not loading: a directory without a config, next to the name with .json",
            APP,
            r#"{"extends":"./base"}"#,
            &[("app/base/readme.md", "nothing"), ("app/base.json", V1)],
            &[],
        ),
        (
            "not loading: a reference to a directory without a config, next to the name with .json",
            APP,
            r#"{"references":[{"path":"./proj"}]}"#,
            &[
                ("app/proj/readme.md", "nothing"),
                (
                    "app/proj.json",
                    r#"{"compilerOptions":{"paths":{"@lib/*":["./v1/*"]}},"include":["src"]}"#,
                ),
            ],
            &[],
        ),
        // The same behind a linked directory: the link above the looked-at place goes.
        (
            "a config behind a linked directory, by a name without .json",
            APP,
            r#"{"extends":"./linked/base"}"#,
            &[("shared/base.json", V1)],
            &[("app/linked", "../shared")],
        ),
    ];
    // What the file's alias resolves to and what is reported on the file.
    let outcome = |root: &Path| -> String {
        let e = Env::at(
            root.to_path_buf(),
            TraceOptions::default(),
            JsResolveOptions::default(),
            DirCache::new(),
        );
        let out = e.resolve(FILE, Import, "@lib/dep");
        format!(
            "{:?} {:?} {:?}",
            out.targets,
            out.uncertainty,
            e.file_uncertainties(FILE)
        )
    };
    // Every file, link and directory of the tree (links are not followed).
    fn walk(
        dir: &Path,
        files: &mut Vec<PathBuf>,
        links: &mut Vec<PathBuf>,
        dirs: &mut Vec<PathBuf>,
    ) {
        dirs.push(dir.to_path_buf());
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap())
            .collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let t = e.file_type().unwrap();
            if t.is_symlink() {
                links.push(e.path());
            } else if t.is_dir() {
                walk(&e.path(), files, links, dirs);
            } else {
                files.push(e.path());
            }
        }
    }
    let mut checked = 0;
    for (name, manifest, config, others, links) in layouts {
        let t = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(t.path()).unwrap();
        let write = |rel: &str, text: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        };
        write("package.json", r#"{"name":"root"}"#);
        write("app/package.json", manifest);
        write("app/tsconfig.json", config);
        write(FILE, "");
        write("app/v1/dep.ts", "");
        write("app/v2/dep.ts", "");
        for (rel, text) in *others {
            write(rel, text);
        }
        for (link, target) in *links {
            let l = root.join(link);
            std::fs::create_dir_all(l.parent().unwrap()).unwrap();
            turborepo_trace_core::os::symlink(target, l).unwrap();
        }
        let before = outcome(&root);
        assert_eq!(
            before.contains("app/v1/dep.ts"),
            !name.starts_with("not loading"),
            "{name}: whether the config loads: {before}"
        );
        let (read, missing) = config_record(&root, FILE);
        let (mut files, mut tree_links, mut dirs) = (Vec::new(), Vec::new(), Vec::new());
        walk(&root, &mut files, &mut tree_links, &mut dirs);
        let rel = |p: &Path| rel2(&root, p).unwrap();

        // A file that changes, or goes (the configs and manifests: what the alias names
        // and the file itself are no part of a load).
        for f in &files {
            if ["app/src", "app/v1", "app/v2"]
                .iter()
                .any(|d| f.starts_with(root.join(d)))
            {
                continue;
            }
            let text = std::fs::read_to_string(f).unwrap();
            let mut variants = vec![None, Some("{".to_owned()), Some("{}".to_owned())];
            if text.contains("v1") {
                variants.push(Some(text.replace("v1", "v2")));
            }
            for v in variants {
                match &v {
                    Some(v) => std::fs::write(f, v).unwrap(),
                    None => std::fs::remove_file(f).unwrap(),
                }
                let after = outcome(&root);
                std::fs::write(f, &text).unwrap();
                checked += 1;
                assert!(
                    after == before || read.contains(&rel(f)),
                    "{name}: {} as {v:?} changes the outcome and is not in the record {read:?}\n  \
                     {before}\n  {after}",
                    rel(f)
                );
            }
        }
        // A link that goes.
        for l in &tree_links {
            let target = std::fs::read_link(l).unwrap();
            std::fs::remove_file(l).unwrap();
            let after = outcome(&root);
            turborepo_trace_core::os::symlink(&target, l).unwrap();
            checked += 1;
            // A package directory is recorded by the store, as a place seen.
            assert!(
                after == before || read.contains(&rel(l)) || rel(l).contains("node_modules"),
                "{name}: without the link {} the outcome changes, and it is not in the record \
                 {read:?}",
                rel(l)
            );
        }
        // A file that appears: a config or a manifest in a directory of the tree, the
        // name of a file without its extension, a directory of that name, and a
        // nearer place of what was found in a `node_modules` directory.
        let mut places: Vec<PathBuf> = Vec::new();
        for d in &dirs {
            places.extend(["tsconfig.json", "jsconfig.json", "package.json"].map(|n| d.join(n)));
        }
        for f in &files {
            if f.extension().is_some_and(|e| e == "json") {
                places.push(f.with_extension(""));
                places.push(f.with_extension("").join("tsconfig.json"));
                places.push(PathBuf::from(format!("{}.json", f.display())));
            }
        }
        for (link, target) in *links {
            if let Some((_, package)) = link.split_once("node_modules/") {
                for (other, _) in *others {
                    let dir = target.trim_start_matches("../");
                    if let Some(rest) = other.strip_prefix(dir) {
                        places.push(root.join(format!("app/src/node_modules/{package}{rest}")));
                        places.push(root.join(format!("app/node_modules/{package}{rest}")));
                    }
                }
            }
        }
        for p in places {
            // (Not where the alias leads: that is the resolution's, not the load's.)
            if p.exists()
                || std::fs::symlink_metadata(&p).is_ok()
                || ["app/v1", "app/v2"]
                    .iter()
                    .any(|d| p.starts_with(root.join(d)))
            {
                continue;
            }
            // The first directory that has to be made for it.
            let made = p
                .ancestors()
                .skip(1)
                .take_while(|a| !a.exists())
                .last()
                .map(Path::to_path_buf);
            if std::fs::create_dir_all(p.parent().unwrap()).is_err() {
                // (Below a file.)
                continue;
            }
            std::fs::write(&p, V2).unwrap();
            let after = outcome(&root);
            match &made {
                Some(m) => std::fs::remove_dir_all(m).unwrap(),
                None => std::fs::remove_file(&p).unwrap(),
            }
            checked += 1;
            let looked = p
                .ancestors()
                .filter_map(|a| rel2(&root, a))
                .any(|a| missing.contains(&a));
            assert!(
                after == before || looked,
                "{name}: {} appears and changes the outcome, and no place at or above it was \
                 looked at: {missing:?}\n  {before}\n  {after}",
                rel(&p)
            );
        }
        // A directory that appears, empty: at the name of a config without its
        // extension (the loader asks whether that name is a directory before it
        // tries the name with `.json`), and at every place that was looked at.
        let mut empty: Vec<PathBuf> = files
            .iter()
            .filter(|f| f.extension().is_some_and(|e| e == "json"))
            .map(|f| f.with_extension(""))
            .collect();
        empty.extend(missing.iter().map(|m| root.join(m)));
        for p in empty {
            if std::fs::symlink_metadata(&p).is_ok() || !p.parent().is_some_and(Path::is_dir) {
                continue;
            }
            std::fs::create_dir(&p).unwrap();
            let after = outcome(&root);
            std::fs::remove_dir(&p).unwrap();
            checked += 1;
            let looked = p
                .ancestors()
                .filter_map(|a| rel2(&root, a))
                .any(|a| missing.contains(&a));
            assert!(
                after == before || looked,
                "{name}: the directory {} appears and changes the outcome, and no place at or \
                 above it was looked at: {missing:?}\n  {before}\n  {after}",
                rel(&p)
            );
        }
        // A directory that goes, with everything in it: what the load read in it, and
        // what it looked for in it and did not find (it looked there because the
        // directory was one). Not the directories the file and its alias targets lie
        // in; a package directory is recorded by the store, as a place seen.
        let hold = root.join(".held");
        for d in dirs.iter().skip(1) {
            let at = rel(d);
            if root.join(FILE).starts_with(d)
                || ["app/v1", "app/v2"].iter().any(|v| root.join(v) == *d)
                || !d.exists()
            {
                continue;
            }
            std::fs::rename(d, &hold).unwrap();
            let after = outcome(&root);
            std::fs::rename(&hold, d).unwrap();
            checked += 1;
            let below = |p: &String| p.strip_prefix(&at).is_some_and(|r| r.starts_with('/'));
            assert!(
                after == before
                    || read.iter().any(below)
                    || missing.iter().any(below)
                    || at.contains("node_modules"),
                "{name}: without the directory {at} the outcome changes, and nothing at or below \
                 it is in the record: read {read:?}, looked for {missing:?}\n  {before}\n  {after}"
            );
        }
    }
    assert!(checked > 600, "{checked} changes checked");
}

/// A package directory that a lookup enters and that does not answer is
/// recorded by its manifest: the resolution view when there is one, a probe
/// when there is none. The search goes on to the levels above it, and a file
/// found there says nothing of the directory passed, so the record does.
#[test]
fn resolver_records_the_manifest_of_a_package_directory_that_does_not_answer() {
    let t = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(t.path()).unwrap();
    let write = |rel: &str, text: &str| {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    };
    write("package.json", r#"{"name":"root"}"#);
    write("app/package.json", r#"{"name":"app"}"#);
    write("app/src/main.js", "require('dep');\n");
    // The nearer directory of the name: no manifest, no index file.
    write("app/node_modules/dep/lib/x.js", "");
    // What one resolution of `dep` from the app records, relative to the tree.
    let observe = || {
        let e = Env::at(
            root.clone(),
            TraceOptions::default(),
            JsResolveOptions::default(),
            DirCache::with_recording(),
        );
        let frame = turborepo_trace_core::record::start();
        let out = e.resolve("app/src/main.js", Require, "dep");
        let obs = frame.finish();
        let rels =
            |v: &[PathBuf]| -> Vec<String> { v.iter().filter_map(|p| rel2(&root, p)).collect() };
        (out, rels(&obs.missing), rels(&obs.view_reads))
    };
    let manifest = "app/node_modules/dep/package.json".to_owned();

    // Nothing answers: the manifest that is not there is a probe, and so is the
    // name at the level above.
    let (out, missing, views) = observe();
    assert_eq!(out.targets, ["unresolved"]);
    assert!(missing.contains(&manifest), "{missing:?}");
    assert!(!views.contains(&manifest), "{views:?}");
    assert!(
        missing.iter().any(|m| m == "node_modules"),
        "the walk goes on: {missing:?}"
    );

    // A manifest without an entry: a view read, although nothing resolved.
    write("app/node_modules/dep/package.json", r#"{"name":"dep"}"#);
    let (out, missing, views) = observe();
    assert_eq!(out.targets, ["unresolved"]);
    assert!(views.contains(&manifest), "{views:?}");
    assert!(!missing.contains(&manifest), "{missing:?}");

    // The name answers a level above: the directory passed is still recorded.
    write(
        "node_modules/dep/package.json",
        r#"{"name":"dep","main":"index.js"}"#,
    );
    write("node_modules/dep/index.js", "");
    let (out, _, views) = observe();
    assert_eq!(out.targets, ["node_modules/dep/index.js"]);
    assert!(views.contains(&manifest), "{views:?}");
    std::fs::remove_file(root.join("app/node_modules/dep/package.json")).unwrap();
    let (out, missing, _) = observe();
    assert_eq!(out.targets, ["node_modules/dep/index.js"]);
    assert!(missing.contains(&manifest), "{missing:?}");

    // The nearer directory answers: its manifest is the resolution's own, and the
    // level above is not asked.
    write(
        "app/node_modules/dep/package.json",
        r#"{"name":"dep","main":"lib/x.js"}"#,
    );
    let (out, missing, _) = observe();
    assert_eq!(out.targets, ["app/node_modules/dep/lib/x.js"]);
    assert!(out.manifests.contains(&manifest), "{:?}", out.manifests);
    assert!(!missing.contains(&manifest), "{missing:?}");
}

fn rel2(root: &Path, p: &Path) -> Option<String> {
    p.strip_prefix(root).ok().map(|r| r.display().to_string())
}

#[test]
fn resolver_externals_stop_workspace_packages() {
    let e = Env::with(
        "workspace/apps/web",
        TraceOptions {
            externals: Externals::Stop,
            ..Default::default()
        },
        JsResolveOptions::default(),
    );
    // Workspace packages linked into `node_modules` are first party (link paths;
    // core canonicalizes them).
    assert_eq!(
        e.targets("index.js", Require, "@ws/lib"),
        ["node_modules/@ws/lib/index.js"]
    );
    let hoisted = e.targets("index.js", Require, "@ws/hoisted");
    assert_eq!(hoisted.len(), 1);
    assert!(
        hoisted[0].ends_with("/workspace/node_modules/@ws/hoisted/index.js"),
        "{hoisted:?}"
    );
    // Real third-party packages: in `node_modules/.pnpm`, or linked from outside
    // the workspace.
    assert_eq!(e.targets("index.js", Require, "dep"), ["external:dep"]);
    assert_eq!(
        e.targets("index.js", Require, "linked"),
        ["external:linked"]
    );
}

#[test]
fn resolver_pnpm_returns_link_paths_for_core_to_canonicalize() {
    let e = Env::new("pnpm");
    let out = e.resolve("index.js", Require, "foo");
    assert_eq!(out.targets, ["node_modules/foo/index.js"]);
    assert_eq!(
        out.manifests,
        ["package.json", "node_modules/foo/package.json"]
    );
    // From foo's real location, `bar` is found in the `.pnpm` sibling directory.
    assert_eq!(
        e.targets(
            "node_modules/.pnpm/foo@1.0.0/node_modules/foo/index.js",
            Require,
            "bar"
        ),
        ["node_modules/.pnpm/foo@1.0.0/node_modules/bar/index.js"]
    );
}

#[test]
fn resolver_externals_stop() {
    let e = Env::with(
        "basic",
        TraceOptions {
            externals: Externals::Stop,
            ..Default::default()
        },
        JsResolveOptions::default(),
    );
    assert_eq!(
        e.targets("main.js", Require, "@scope/pkg"),
        ["external:@scope/pkg"]
    );
    assert_eq!(
        e.targets("main.js", Require, "pkg-exports/features/a"),
        ["external:pkg-exports"]
    );
    // Project files, self-references and builtins are not external.
    assert_eq!(
        e.targets("main.js", Require, "app/feature"),
        ["src/feature.js"]
    );
    assert_eq!(e.targets("main.js", Require, "./src/rel"), ["src/rel.js"]);
    assert_eq!(e.targets("main.js", Require, "fs"), ["builtin:node:fs"]);
}

#[test]
fn resolver_non_module_refs_and_specifier_forms() {
    let e = Env::new("basic");
    // fs reads resolve as plain paths, without extension probing.
    assert_eq!(
        e.targets("main.js", RefKind::FsRead, "./data.json"),
        ["data.json"]
    );
    // A missing path is a normal outcome for fs probes (`existsSync`): a quiet
    // record.
    let out = e.resolve("main.js", RefKind::FsRead, "./src/rel");
    assert_eq!(out.targets, ["unresolved"]);
    assert_eq!(out.uncertainty, None);
    // Directories expand to every file below them (nft's `emitAssetDirectory`).
    let dir = e.targets("main.js", RefKind::FsReadDir, "./src/dir");
    assert_eq!(dir.len(), 1);
    assert!(dir[0].starts_with("glob:") && dir[0].ends_with("/src/dir/**"));
    // Alternatives resolve each branch.
    let r = RawRef::new(
        Require,
        Specifier::Alternatives(
            [
                Specifier::literal("./src/rel"),
                Specifier::literal("pkg-main"),
            ]
            .into_iter()
            .collect(),
        ),
        Span::new(0, 1),
    );
    let out = e.resolve_ref("main.js", r);
    assert_eq!(
        out.targets,
        ["src/rel.js", "node_modules/pkg-main/lib/entry.js"]
    );
    // Both resolved: each is a target, nothing about the reference is unknown.
    assert_eq!(out.uncertainty, None);
    // One that does not resolve keeps its own record.
    let r = RawRef::new(
        Require,
        Specifier::Alternatives(
            [
                Specifier::literal("./src/rel"),
                Specifier::literal("./src/no-such-module"),
            ]
            .into_iter()
            .collect(),
        ),
        Span::new(0, 1),
    );
    let out = e.resolve_ref("main.js", r);
    assert_eq!(out.targets, ["src/rel.js"]);
    assert_eq!(out.uncertainty, Some(UncertaintyKind::UnresolvedSpecifier));
    // Unknown specifiers.
    let out = e.resolve_ref(
        "main.js",
        RawRef::new(Require, Specifier::Unknown, Span::new(0, 1)).with_flags(RefFlags::OPTIONAL),
    );
    assert_eq!(out.targets, ["unresolved"]);
    assert_eq!(out.uncertainty, Some(UncertaintyKind::ComputedSpecifier));
}

// ---- End-to-end snapshots through the real extractor. ----

fn case(name: &str) -> String {
    fixtures().join(name).to_string_lossy().into_owned()
}

#[test]
fn snapshot_basic() {
    fixture_snapshot!(
        "resolve_basic",
        Tracer::builder().language(JsLanguage),
        case("basic"),
        &["main.js", "esm.mjs"],
    );
}

#[test]
fn snapshot_ts() {
    fixture_snapshot!(
        "resolve_ts",
        Tracer::builder().language(JsLanguage),
        case("ts"),
        &["src/main.ts"],
    );
}

#[test]
fn snapshot_ts_include_types() {
    fixture_snapshot!(
        "resolve_ts_include_types",
        Tracer::builder().language(JsLanguage).include_types(true),
        case("ts"),
        &["src/main.ts"],
    );
}

#[test]
fn snapshot_pnpm() {
    fixture_snapshot!(
        "resolve_pnpm",
        Tracer::builder().language(JsLanguage),
        case("pnpm"),
        &["index.js"],
    );
}

#[test]
fn snapshot_pnpm_externals_stop() {
    fixture_snapshot!(
        "resolve_pnpm_externals_stop",
        Tracer::builder()
            .language(JsLanguage)
            .externals(Externals::Stop),
        case("pnpm"),
        &["index.js"],
    );
}

#[test]
fn snapshot_tsconfig_js() {
    fixture_snapshot!(
        "resolve_tsconfig_js",
        Tracer::builder().language(JsLanguage),
        case("tsconfig-js"),
        &["src/main.ts", "jsapp/src/index.js"],
    );
}

#[test]
fn snapshot_tsconfig_broken() {
    fixture_snapshot!(
        "resolve_tsconfig_broken",
        Tracer::builder().language(JsLanguage),
        case("tsconfig-broken"),
        &["src/main.ts"],
    );
}

#[test]
fn snapshot_workspace_externals_stop() {
    fixture_snapshot!(
        "resolve_workspace_externals_stop",
        Tracer::builder()
            .language(JsLanguage)
            .externals(Externals::Stop),
        case("workspace"),
        &["apps/web/index.js"],
    );
}

#[test]
fn pnpm_nft_output() {
    let root = fixtures().join("pnpm");
    let trace = Tracer::builder()
        .base(&root)
        .language(JsLanguage)
        .build()
        .unwrap()
        .trace(&[root.join("index.js")])
        .unwrap();
    let mut out = Vec::new();
    turborepo_trace_core::output::write_nft(&trace, &Default::default(), &mut out).unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        concat!(
            r#"{"version":1,"files":["index.js","#,
            r#""node_modules/.pnpm/bar@2.0.0/node_modules/bar/index.js","#,
            r#""node_modules/.pnpm/bar@2.0.0/node_modules/bar/package.json","#,
            r#""node_modules/.pnpm/foo@1.0.0/node_modules/bar","#,
            r#""node_modules/.pnpm/foo@1.0.0/node_modules/foo/index.js","#,
            r#""node_modules/.pnpm/foo@1.0.0/node_modules/foo/package.json","#,
            r#""node_modules/foo","package.json"],"#,
            r#""symlinks":{"3":"node_modules/.pnpm/bar@2.0.0/node_modules/bar","#,
            r#""6":"node_modules/.pnpm/foo@1.0.0/node_modules/foo"}}"#,
            "\n"
        )
    );
}

#[test]
fn deterministic_across_threads() {
    let render = |threads| {
        turborepo_trace_testkit::fixture_json(
            Tracer::builder().language(JsLanguage).threads(threads),
            case("basic"),
            &["main.js", "esm.mjs"],
        )
    };
    let one = render(1);
    for t in [2, 8] {
        assert_eq!(one, render(t));
    }
}
