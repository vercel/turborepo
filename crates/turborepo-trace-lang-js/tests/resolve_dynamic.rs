//! M2 resolution tests: partially known specifiers, alternatives, fs reads
//! (file dir and cwd), readdir, workers, child processes, assets, package
//! special cases, native addons and their shared libraries, files outside the
//! base, `--all-conditions`, and the `summary` and `list` outputs.
//!
//! The trees live in `fixtures/js/resolve/{dynamic,special,outside}`. Their JS
//! sources show the code each reference stands for, but references are fed by a
//! scripted language ([`Scripted`]) with hand-built [`RawRef`]s in the shapes
//! the extractor emits, so these tests exercise the resolver independently of
//! extraction. Spans point at the code in the sources.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
    sync::Arc,
};

use turborepo_trace_core::{
    Base, Externals, ExtractCx, LangId, Language, Part, ProjectCx, RawRef, RefKind, RefSink,
    Resolver, Span, Specifier, Trace, Tracer, TracerBuilder, Uncertainty, UncertaintyKind, WhyVia,
    fs::normalize,
    output::{self, OutputOptions},
};
use turborepo_trace_lang_js::{
    JsLanguage,
    resolve::{JsResolveOptions, JsResolver},
};
use turborepo_trace_testkit::redact_base;

fn fixtures() -> PathBuf {
    normalize(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../turborepo-trace-testkit/fixtures/js/resolve"),
    )
}

// ---- Scripted language
// -------------------------------------------------------------------

/// References per absolute file path.
type Refs = HashMap<PathBuf, Vec<RawRef<'static>>>;

/// A JS language whose extractor returns scripted references; resolution is the
/// real [`JsResolver`].
struct Scripted(Arc<Refs>);

impl Language for Scripted {
    fn id(&self) -> LangId {
        LangId("js")
    }

    fn extensions(&self) -> &'static [&'static str] {
        JsLanguage.extensions()
    }

    fn extract<'s>(&self, _src: &'s [u8], cx: &ExtractCx<'_>, out: &mut RefSink<'s>) {
        for r in self.0.get(cx.path).into_iter().flatten() {
            out.push(r.clone());
        }
    }

    fn resolver(&self, project: &ProjectCx<'_>) -> Box<dyn Resolver> {
        Box::new(JsResolver::new(project, &JsResolveOptions::default()))
    }
}

/// Builds scripted references for one fixture.
struct Script {
    root: PathBuf,
    refs: Refs,
}

impl Script {
    fn new(case: &str) -> Self {
        Script {
            root: fixtures().join(case),
            refs: HashMap::new(),
        }
    }

    /// Adds `r` to `file`, with its span (and its uncertainty's) set to the
    /// first occurrence of `needle` in the file.
    fn add(&mut self, file: &str, needle: &str, mut r: RawRef<'static>) -> &mut Self {
        let path = self.root.join(file);
        let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{file}: {e}"));
        let start = src
            .find(needle)
            .unwrap_or_else(|| panic!("{needle:?} not in {file}"));
        r.span = Span::new(start as u32, (start + needle.len()) as u32);
        if let Some(u) = &mut r.uncertainty {
            u.span = r.span;
        }
        self.refs.entry(path).or_default().push(r);
        self
    }

    fn lang(&self) -> Scripted {
        Scripted(Arc::new(self.refs.clone()))
    }
}

fn lit(s: &str) -> Specifier<'static> {
    Specifier::Literal(s.to_string().into())
}

/// A pattern; `*` elements are wildcards.
fn pat(parts: &[&str]) -> Specifier<'static> {
    Specifier::Pattern(
        parts
            .iter()
            .map(|p| match *p {
                "*" => Part::Wild { reason: "x".into() },
                s => Part::Lit(s.to_string().into()),
            })
            .collect(),
    )
}

fn r(kind: RefKind, spec: Specifier<'static>) -> RawRef<'static> {
    RawRef::new(kind, spec, Span::DUMMY)
}

fn unc(kind: UncertaintyKind, detail: &str) -> Uncertainty {
    Uncertainty::new(kind, Span::DUMMY, detail)
}

use RefKind::{Asset, ChildProcess, FsRead, FsReadDir, Require, Worker};
use UncertaintyKind::{CwdDependent, PartialSpecifier};

// ---- Running traces
// ----------------------------------------------------------------------

struct Run {
    root: PathBuf,
    trace: Trace,
}

impl Run {
    fn new(builder: TracerBuilder, base: &Path, cwd: &Path, entries: &[&str]) -> Self {
        let tracer = builder.base(base).cwd(cwd).build().expect("tracer");
        let entries: Vec<PathBuf> = entries.iter().map(|e| base.join(e)).collect();
        let trace = tracer.trace(&entries).expect("trace");
        Run {
            root: base.to_path_buf(),
            trace,
        }
    }

    fn rel(&self, p: &Path) -> String {
        p.strip_prefix(&self.root)
            .unwrap_or(p)
            .to_string_lossy()
            .into_owned()
    }

    fn files(&self) -> BTreeSet<String> {
        self.trace.files().map(|p| self.rel(p)).collect()
    }

    fn has(&self, f: &str) -> bool {
        self.files().contains(f)
    }

    fn reasons(&self, f: &str) -> Vec<&'static str> {
        let id = self.trace.file_id(&self.root.join(f)).expect(f);
        self.trace.reason(id).unwrap().kinds.names().collect()
    }

    /// `(file, kind, spec, detail)` for every uncertainty.
    fn uncertainties(&self) -> Vec<(String, UncertaintyKind, String, String)> {
        self.trace
            .uncertainties()
            .into_iter()
            .map(|(f, u, e)| {
                (
                    self.rel(self.trace.path(f).unwrap()),
                    u.kind,
                    e.map(|e| e.spec.to_string()).unwrap_or_default(),
                    u.detail.to_string(),
                )
            })
            .collect()
    }

    fn uncertainty_for(&self, spec: &str) -> Option<(UncertaintyKind, String)> {
        self.uncertainties()
            .into_iter()
            .find(|(_, _, s, _)| s == spec)
            .map(|(_, k, _, d)| (k, d))
    }

    fn json(&self, redact: &Path) -> String {
        let mut out = Vec::new();
        output::write_json(&self.trace, &mut out).unwrap();
        redact_base(&String::from_utf8(out).unwrap(), redact)
    }

    fn summary(&self, redact: &Path) -> String {
        let mut out = Vec::new();
        output::write_summary(&self.trace, &mut out).unwrap();
        String::from_utf8(out)
            .unwrap()
            .replace(&redact.to_string_lossy().into_owned(), "<fixture>")
    }
}

// ---- dynamic: patterns, alternatives, fs, workers, assets
// --------------------------------

fn dynamic_script() -> Script {
    let mut s = Script::new("dynamic");
    let m = "main.js";
    s.add(m, "require('path')", r(Require, lit("path")))
        .add(
            m,
            "require('./lib/helper')",
            r(Require, lit("./lib/helper")),
        )
        .add(
            m,
            "require('./locales/' + lang + '.json')",
            r(Require, pat(&["./locales/", "*", ".json"]))
                .with_uncertainty(unc(PartialSpecifier, "lang")),
        )
        .add(
            m,
            "require('./plugins/' + name)",
            r(Require, pat(&["./plugins/", "*"])),
        )
        .add(
            m,
            "require('./plugins/' + name + '/index')",
            r(Require, pat(&["./plugins/", "*", "/index"])),
        )
        .add(
            m,
            "require('./nothing/' + name + '.js')",
            r(Require, pat(&["./nothing/", "*", ".js"])),
        )
        .add(
            m,
            "require(fast ? './impl/fast' : './impl/slow')",
            r(
                Require,
                Specifier::Alternatives(vec![lit("./impl/fast"), lit("./impl/slow")]),
            ),
        )
        .add(
            m,
            "fs.readFileSync(path.join(__dirname, 'data/config.json'))",
            r(FsRead, lit("./data/config.json")),
        )
        .add(
            m,
            "fs.readFileSync(path.join(__dirname, 'data', kind + '.json'))",
            r(FsRead, pat(&["./data/", "*", ".json"]))
                .with_uncertainty(unc(PartialSpecifier, "kind")),
        )
        .add(
            m,
            "fs.readdirSync(path.join(__dirname, 'templates'))",
            r(FsReadDir, lit("./templates")),
        )
        .add(
            m,
            "fs.existsSync(path.join(__dirname, 'data/optional.json'))",
            r(FsRead, lit("./data/optional.json")),
        )
        .add(
            m,
            "fs.readFileSync(path.join(__dirname, name))",
            r(FsRead, pat(&["./", "*"])).with_uncertainty(unc(PartialSpecifier, "name")),
        )
        .add(
            m,
            "fs.readFileSync('settings.json')",
            r(FsRead, lit("settings.json"))
                .with_base(Base::Cwd)
                .with_uncertainty(unc(CwdDependent, "relative to the process cwd")),
        )
        .add(
            m,
            "fs.readFileSync('conf/' + env + '.yml')",
            r(FsRead, pat(&["conf/", "*", ".yml"]))
                .with_base(Base::Cwd)
                .with_uncertainty(unc(CwdDependent, "relative to the process cwd")),
        )
        .add(
            m,
            "new Worker(path.join(__dirname, 'worker.js'))",
            r(Worker, lit("./worker.js")),
        )
        .add(
            m,
            "fork(path.join(__dirname, 'child'))",
            r(ChildProcess, lit("./child")),
        )
        .add(
            m,
            "new URL('./assets/logo.png', import.meta.url)",
            r(Asset, lit("./assets/logo.png")),
        );
    s.add(
        "lib/helper.js",
        "fs.readFileSync(path.join(__dirname, '../data/' + name + '.json'))",
        r(FsRead, pat(&["../data/", "*", ".json"])),
    );
    s.add(
        "child.js",
        "require('./lib/helper')",
        r(Require, lit("./lib/helper")),
    );
    s
}

fn dynamic_run(builder: TracerBuilder) -> Run {
    let root = fixtures().join("dynamic");
    Run::new(
        builder.language(dynamic_script().lang()),
        &root,
        &root.join("runtime"),
        &["main.js"],
    )
}

#[test]
fn dynamic_module_patterns_glob_and_follow() {
    let run = dynamic_run(Tracer::builder());
    for f in ["locales/en.json", "locales/de.json", "locales/sub/fr.json"] {
        assert!(run.has(f), "{f}");
    }
    // nft's glob ignores nested node_modules and README/CHANGELOG files.
    assert!(!run.has("locales/node_modules/junk.json"));
    assert!(!run.has("locales/README.md"));
    for f in ["plugins/a.js", "plugins/b.js", "plugins/c/index.js"] {
        assert!(run.has(f), "{f}");
        assert!(
            run.trace
                .is_parsed(run.trace.file_id(&run.root.join(f)).unwrap()),
            "{f} is followed as code"
        );
    }
    assert!(!run.has("plugins/node_modules/x/index.js"));
    assert!(!run.has("plugins/README.md"));
    // The extractor's uncertainty is kept, with the glob result appended.
    let (kind, detail) = run.uncertainty_for("./locales/*.json").unwrap();
    assert_eq!(kind, PartialSpecifier);
    assert!(
        detail.starts_with("lang; ") && detail.ends_with("3 match(es)"),
        "{detail}"
    );
    // `/index` without an extension tries the resolver's extensions.
    let (_, detail) = run.uncertainty_for("./plugins/*/index").unwrap();
    assert!(detail.contains("?(ext): 1 match(es)"), "{detail}");
    // No match: unresolved.
    let (kind, _) = run.uncertainty_for("./nothing/*.js").unwrap();
    assert_eq!(kind, UncertaintyKind::UnresolvedSpecifier);
    // Alternatives: both branches. Each one resolved, so nothing about the
    // reference is unknown and it has no record.
    assert!(run.has("impl/fast.js") && run.has("impl/slow.js"));
    assert!(run.uncertainty_for("./impl/fast | ./impl/slow").is_none());
}

#[test]
fn dynamic_fs_reads_readdir_and_cwd() {
    let run = dynamic_run(Tracer::builder());
    // `__dirname` reads: literal and pattern (from main.js and from lib/helper.js).
    assert!(run.has("data/config.json"));
    assert!(run.has("data/users.json"));
    assert!(!run.has("data/notes.txt"));
    assert_eq!(run.reasons("data/config.json"), ["asset"]);
    // readdir: every file below, as nft, minus nested node_modules and READMEs.
    for f in [
        "templates/a.html",
        "templates/b.html",
        "templates/partials/c.html",
    ] {
        assert!(run.has(f), "{f}");
    }
    assert!(!run.has("templates/node_modules/x.html"));
    assert!(!run.has("templates/README.md"));
    // A probe of a missing file is quiet: an edge, but no uncertainty.
    assert!(run.uncertainty_for("./data/optional.json").is_none());
    assert!(
        run.trace
            .edges()
            .iter()
            .any(|e| e.spec.as_ref() == "./data/optional.json"
                && matches!(e.to, turborepo_trace_core::Target::Unresolved))
    );
    // `path.join(__dirname, name)` would be the whole directory: refused, as nft.
    let (kind, detail) = run.uncertainty_for("./*").unwrap();
    assert_eq!(kind, UncertaintyKind::PatternTooBroad);
    assert!(detail.contains("own directory"), "{detail}");
    // cwd-relative reads resolve against --cwd and keep CwdDependent.
    assert!(run.has("runtime/settings.json"));
    assert!(run.has("runtime/conf/prod.yml") && run.has("runtime/conf/dev.yml"));
    let (kind, detail) = run.uncertainty_for("conf/*.yml").unwrap();
    assert_eq!(kind, CwdDependent);
    assert!(detail.ends_with("2 match(es)"), "{detail}");
    assert_eq!(
        run.uncertainty_for("settings.json").unwrap().0,
        CwdDependent
    );
}

#[test]
fn dynamic_workers_children_assets_and_scopes() {
    let run = dynamic_run(Tracer::builder());
    let parsed = |f: &str| {
        run.trace
            .is_parsed(run.trace.file_id(&run.root.join(f)).unwrap())
    };
    assert!(run.has("worker.js") && parsed("worker.js"));
    // `fork('./child')` probes extensions like `require`.
    assert!(run.has("child.js") && parsed("child.js"));
    assert_eq!(run.reasons("worker.js"), ["dependency"]);
    assert!(run.has("assets/logo.png") && !parsed("assets/logo.png"));
    assert_eq!(run.reasons("assets/logo.png"), ["asset"]);
    // worker.js has no references but still records its package scope (nft).
    assert!(run.trace.edges().iter().any(|e| {
        e.kind == RefKind::Manifest && run.rel(run.trace.path(e.from).unwrap()) == "worker.js"
    }));
}

#[test]
fn dynamic_glob_limit() {
    let run = dynamic_run(Tracer::builder().max_glob(2));
    let (kind, detail) = run.uncertainty_for("./locales/*.json").unwrap();
    assert_eq!(kind, UncertaintyKind::PatternTooBroad);
    assert!(detail.contains("more than 2"), "{detail}");
    let locales = run
        .files()
        .into_iter()
        .filter(|f| f.starts_with("locales/"))
        .count();
    assert_eq!(locales, 2);
}

#[test]
fn snapshot_dynamic() {
    let run = dynamic_run(Tracer::builder());
    insta::assert_snapshot!("resolve_dynamic", run.json(&run.root));
}

#[test]
fn snapshot_dynamic_summary() {
    let run = dynamic_run(Tracer::builder());
    insta::assert_snapshot!("resolve_dynamic_summary", run.summary(&run.root));
}

// ---- special: package special cases, native addons, shared libraries
// ---------------------

fn special_script() -> Script {
    let mut s = Script::new("special");
    for p in [
        "better-sqlite3",
        "gyp-pkg",
        "pregyp-pkg",
        "abi-pkg",
        "sharp",
        "esbuild",
        "@prisma/client",
        "msgpackr-extract",
        "semver",
        "direct-addon",
        "nobin-pkg",
    ] {
        s.add("main.js", &format!("require('{p}')"), r(Require, lit(p)));
    }
    let nm = "node_modules/";
    s.add(
        &format!("{nm}better-sqlite3/lib/index.js"),
        "require('bindings')",
        r(Require, lit("bindings")),
    )
    .add(
        &format!("{nm}better-sqlite3/lib/index.js"),
        "require('./binding')",
        r(Require, lit("./binding")),
    )
    .add(
        &format!("{nm}better-sqlite3/lib/binding.js"),
        "require(filename)",
        r(Require, Specifier::Unknown),
    )
    .add(
        &format!("{nm}gyp-pkg/index.js"),
        "require('node-gyp-build')",
        r(Require, lit("node-gyp-build")),
    )
    .add(
        &format!("{nm}pregyp-pkg/lib/index.js"),
        "require('@mapbox/node-pre-gyp')",
        r(Require, lit("@mapbox/node-pre-gyp")),
    )
    .add(
        &format!("{nm}pregyp-pkg/lib/index.js"),
        "require(p)",
        r(Require, Specifier::Unknown),
    )
    .add(
        &format!("{nm}abi-pkg/index.js"),
        "require('@mapbox/node-pre-gyp')",
        r(Require, lit("@mapbox/node-pre-gyp")),
    )
    .add(
        &format!("{nm}sharp/lib/index.js"),
        "require('./sharp')",
        r(Require, lit("./sharp")),
    )
    .add(
        &format!("{nm}sharp/lib/sharp.js"),
        "require(`@img/sharp-${runtimePlatform}/sharp.node`)",
        r(Require, pat(&["@img/sharp-", "*", "/sharp.node"]))
            .with_uncertainty(unc(PartialSpecifier, "runtimePlatform")),
    )
    .add(
        &format!("{nm}@prisma/client/index.js"),
        "require('.prisma/client/index')",
        r(Require, lit(".prisma/client/index")),
    )
    .add(
        &format!("{nm}msgpackr-extract/index.js"),
        "require('node-gyp-build-optional-packages')",
        r(Require, lit("node-gyp-build-optional-packages")),
    )
    .add(
        &format!("{nm}direct-addon/index.js"),
        "require('./build/Release/direct.node')",
        r(Require, lit("./build/Release/direct.node")),
    )
    .add(
        &format!("{nm}nobin-pkg/index.js"),
        "require('bindings')",
        r(Require, lit("bindings")),
    );
    s
}

fn special_run(target: &str) -> Run {
    let root = fixtures().join("special");
    Run::new(
        Tracer::builder()
            .language(special_script().lang())
            .target(target),
        &root,
        &root,
        &["main.js"],
    )
}

#[test]
fn special_native_loaders_linux() {
    let run = special_run("linux/x64");
    let nm = |f: &str| format!("node_modules/{f}");
    // bindings: the build directories, not intermediate obj.target copies.
    assert!(run.has(&nm("better-sqlite3/build/Release/better_sqlite3.node")));
    assert!(!run.has(&nm(
        "better-sqlite3/build/Release/obj.target/better_sqlite3.node"
    )));
    assert_eq!(
        run.reasons(&nm("better-sqlite3/build/Release/better_sqlite3.node")),
        ["asset"]
    );
    // Shared libraries shipped in the addon's package.
    assert!(run.has(&nm("better-sqlite3/build/Release/libhelper.so.1")));
    assert_eq!(
        run.reasons(&nm("better-sqlite3/build/Release/libhelper.so.1")),
        ["sharedlib"]
    );
    assert!(!run.has(&nm("better-sqlite3/deps/sqlite3.c")));
    // better-sqlite3 >= 12 (`lib/binding.js`): the platform prebuild.
    assert!(run.has(&nm("better-sqlite3/prebuilds/linux-x64.node")));
    assert!(!run.has(&nm("better-sqlite3/prebuilds/linuxmusl-x64.node")));
    assert!(!run.has(&nm("better-sqlite3/prebuilds/darwin-arm64.node")));
    // node-gyp-build: prebuilds for the target platform and libc only.
    assert!(run.has(&nm("gyp-pkg/prebuilds/linux-x64/node.napi.glibc.node")));
    assert!(!run.has(&nm("gyp-pkg/prebuilds/linux-x64/node.napi.musl.node")));
    assert!(!run.has(&nm("gyp-pkg/prebuilds/darwin-arm64/node.napi.node")));
    // node-pre-gyp: `binary.module_path` expanded for the target.
    assert!(run.has(&nm("pregyp-pkg/lib/binding/napi-v6-linux-x64/addon.node")));
    assert!(!run.has(&nm("pregyp-pkg/lib/binding/napi-v6-darwin-arm64/addon.node")));
    // `{node_abi}` is not known statically: a pattern plus a partial-specifier
    // record.
    assert!(run.has(&nm("abi-pkg/lib/binding/node-v115-linux-x64/abi.node")));
    assert!(run.has(&nm("abi-pkg/lib/binding/node-v127-linux-x64/abi.node")));
    let partial = run
        .uncertainties()
        .into_iter()
        .find(|(f, _, _, d)| f.ends_with("abi-pkg/index.js") && d.contains("node_abi"))
        .expect("partial node_abi record");
    assert_eq!(partial.1, PartialSpecifier);
    // node-gyp-build-optional-packages: the platform package.
    assert!(run.has(&nm(
        "@msgpackr-extract/msgpackr-extract-linux-x64/node.abi115.node"
    )));
    // A loader without any addon on disk is reported.
    let nobin = run
        .uncertainties()
        .into_iter()
        .find(|(f, _, _, _)| f.ends_with("nobin-pkg/index.js"))
        .expect("nobin record");
    assert_eq!(nobin.1, UncertaintyKind::UnresolvedSpecifier);
    assert!(nobin.3.contains("no native addon"), "{}", nobin.3);
    // A directly required addon pulls in its package's shared libraries, not nested
    // node_modules or other platforms' libraries.
    assert!(run.has(&nm("direct-addon/build/Release/direct.node")));
    assert!(run.has(&nm("direct-addon/lib/libdirect.so")));
    assert!(!run.has(&nm("direct-addon/lib/libdirect.dylib")));
    assert!(!run.has(&nm("direct-addon/node_modules/sub/libnested.so")));
}

#[test]
fn special_file_cases_linux() {
    let run = special_run("linux/x64");
    let nm = |f: &str| format!("node_modules/{f}");
    // sharp: platform packages from optionalDependencies, nested (libvips).
    assert!(run.has(&nm("@img/sharp-linux-x64/lib/sharp-linux-x64.node")));
    assert!(run.has(&nm("@img/sharp-libvips-linux-x64/lib/libvips-cpp.so.42")));
    assert!(run.has(&nm("@img/sharp-libvips-linux-x64/versions.json")));
    assert!(!run.has(&nm("@img/sharp-darwin-arm64/lib/sharp-darwin-arm64.node")));
    // esbuild: its platform binary package.
    assert!(run.has(&nm("@esbuild/linux-x64/bin/esbuild")));
    // Prisma: schema and the engine for the target.
    assert!(run.has(&nm(".prisma/client/schema.prisma")));
    assert!(run.has(&nm(
        ".prisma/client/libquery_engine-debian-openssl-3.0.x.so.node"
    )));
    assert!(!run.has(&nm(
        ".prisma/client/libquery_engine-linux-arm64-openssl-3.0.x.so.node"
    )));
    assert!(!run.has(&nm(".prisma/client/libquery_engine-darwin-arm64.dylib.node")));
    // semver: preload.js.
    assert!(run.has(&nm("semver/preload.js")));
}

#[test]
fn special_cases_follow_the_target_platform() {
    let run = special_run("darwin/arm64");
    let nm = |f: &str| format!("node_modules/{f}");
    assert!(run.has(&nm("gyp-pkg/prebuilds/darwin-arm64/node.napi.node")));
    assert!(!run.has(&nm("gyp-pkg/prebuilds/linux-x64/node.napi.glibc.node")));
    assert!(run.has(&nm("pregyp-pkg/lib/binding/napi-v6-darwin-arm64/addon.node")));
    assert!(run.has(&nm("@img/sharp-darwin-arm64/lib/sharp-darwin-arm64.node")));
    assert!(!run.has(&nm("@img/sharp-linux-x64/lib/sharp-linux-x64.node")));
    assert!(run.has(&nm(".prisma/client/libquery_engine-darwin-arm64.dylib.node")));
    assert!(run.has(&nm("direct-addon/lib/libdirect.dylib")));
    assert!(run.has(&nm("better-sqlite3/prebuilds/darwin-arm64.node")));
    // No esbuild binary for darwin-arm64 is installed in the fixture.
    assert!(!run.has(&nm("@esbuild/linux-x64/bin/esbuild")));
}

#[test]
fn why_explains_special_case_files() {
    let run = special_run("linux/x64");
    let entry = run.root.join("main.js");
    let lib = run
        .root
        .join("node_modules/@img/sharp-libvips-linux-x64/lib/libvips-cpp.so.42");
    let chain = run.trace.why(&entry, &lib).expect("libvips is reachable");
    let last = chain.last().unwrap();
    let WhyVia::Edge(e) = last.via else {
        panic!("edge expected")
    };
    assert!(e.spec.starts_with("special-case sharp"), "{}", e.spec);
    assert!(matches!(e.kind, RefKind::Asset | RefKind::SharedLib));
    let lib = run
        .root
        .join("node_modules/better-sqlite3/build/Release/libhelper.so.1");
    let chain = run.trace.why(&entry, &lib).unwrap();
    let WhyVia::Edge(e) = chain.last().unwrap().via else {
        panic!("edge expected")
    };
    assert_eq!(e.kind, RefKind::SharedLib);
}

#[test]
fn snapshot_special() {
    let run = special_run("linux/x64");
    insta::assert_snapshot!("resolve_special", run.json(&run.root));
}

#[test]
fn snapshot_special_summary() {
    let run = special_run("linux/x64");
    insta::assert_snapshot!("resolve_special_summary", run.summary(&run.root));
}

// ---- outside the base
// -------------------------------------------------------------------

#[test]
fn outside_base_is_reported_once_per_file() {
    let root = fixtures().join("outside");
    let app = root.join("app");
    let run = Run::new(
        Tracer::builder().language(JsLanguage),
        &app,
        &app,
        &["main.js"],
    );
    let outside: Vec<_> = run
        .uncertainties()
        .into_iter()
        .filter(|u| u.1 == UncertaintyKind::OutsideBase)
        .collect();
    // index.js and its package.json, each once, on the referencing file.
    assert_eq!(outside.len(), 2, "{outside:?}");
    assert!(outside.iter().all(|u| u.0 == "main.js"), "{outside:?}");
    assert!(
        outside[0].3.contains("node_modules/shared") && outside[0].3.contains("via 'shared'"),
        "{outside:?}"
    );
    // nft output omits them and reports them.
    let mut out = Vec::new();
    let report = output::write_nft(&run.trace, &OutputOptions::default(), &mut out).unwrap();
    assert_eq!(report.outside_base.len(), 2);
    insta::assert_snapshot!("resolve_outside", run.json(&root));
}

// ---- options and outputs
// ----------------------------------------------------------------

#[test]
fn all_conditions_adds_other_exports_targets() {
    let root = fixtures().join("basic");
    let lang = || {
        let mut s = Script::new("basic");
        s.add("main.js", "pkg-exports", r(Require, lit("pkg-exports")));
        s.lang()
    };
    let run = Run::new(
        Tracer::builder().language(lang()),
        &root,
        &root,
        &["main.js"],
    );
    assert!(run.has("node_modules/pkg-exports/cjs/index.cjs"));
    assert!(!run.has("node_modules/pkg-exports/esm/index.mjs"));
    let run = Run::new(
        Tracer::builder().language(lang()).all_conditions(true),
        &root,
        &root,
        &["main.js"],
    );
    assert!(run.has("node_modules/pkg-exports/cjs/index.cjs"));
    assert!(run.has("node_modules/pkg-exports/esm/index.mjs"));
    assert!(run.has("node_modules/pkg-exports/legacy.js"));
}

#[test]
fn list_output_names_externals() {
    let root = fixtures().join("pnpm");
    let run = Run::new(
        Tracer::builder()
            .language(JsLanguage)
            .externals(Externals::Stop),
        &root,
        &root,
        &["index.js"],
    );
    let mut out = Vec::new();
    output::write_list(&run.trace, &OutputOptions::default(), &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert_eq!(text, "index.js\npackage.json\nexternal:foo\n");
}

#[test]
fn platform_comes_from_target() {
    let fs = Arc::new(turborepo_trace_core::DirCache::new());
    let options = turborepo_trace_core::TraceOptions {
        target: Some("darwin/arm64".into()),
        ..Default::default()
    };
    let r = JsResolver::new(
        &ProjectCx {
            options: &options,
            fs: &fs,
        },
        &JsResolveOptions::default(),
    );
    assert_eq!(r.platform().node_os(), "darwin");
    assert_eq!(r.platform().node_arch(), "arm64");
}
