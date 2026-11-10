//! End-to-end tests: entries -> scheduler -> extract -> resolve -> graph ->
//! outputs, using the stub language and `fixtures/stub/basic`.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use turborepo_trace_core::{
    PhaseFilter, ReasonKinds, RefKind, Target, Trace, Tracer, TracerBuilder, UncertaintyKind,
    WhyVia,
    output::{self, Format, OutputOptions, Selection},
};
use turborepo_trace_testkit::stub::StubLanguage;

fn fixture() -> PathBuf {
    turborepo_trace_core::fs::normalize(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../turborepo-trace-testkit/fixtures/stub/basic"),
    )
}

fn builder(threads: usize) -> TracerBuilder {
    Tracer::builder()
        .base(fixture())
        .threads(threads)
        .language(StubLanguage)
}

fn trace(b: TracerBuilder, entries: &[&str]) -> Trace {
    let root = fixture();
    let entries: Vec<PathBuf> = entries.iter().map(|e| root.join(e)).collect();
    b.build().unwrap().trace(&entries).unwrap()
}

fn rel_files<'a>(t: &Trace, files: impl Iterator<Item = &'a Path>) -> Vec<String> {
    files
        .map(|p| t.relative(p).unwrap().to_string_lossy().into_owned())
        .collect()
}

fn render(t: &Trace, format: Format) -> String {
    let mut out = Vec::new();
    output::write(t, format, &OutputOptions::default(), &mut out).unwrap();
    String::from_utf8(out).unwrap()
}

const MAIN_FILES: &[&str] = &[
    "assets/b.txt",
    "assets/img/logo.png",
    "build-data.txt",
    "build-only.stub",
    "data/config.txt",
    "data/space file.txt",
    "lib/index-data.txt",
    "lib/index.stub",
    "lib/util.stub",
    "locales/de.json",
    "locales/en.json",
    "locales/extra/fr.json",
    "main.stub",
    "shared.txt",
    "stub_modules/pkg/index.stub",
    "stub_modules/pkg/inner-data.txt",
    "stub_modules/pkg/inner.stub",
    "stub_modules/pkg/stub.json",
];

const SECOND_FILES: &[&str] = &[
    "bin/tool",
    "bin/tool-data.txt",
    "data/config.txt",
    "data/link.txt",
    "lib/util.stub",
    "only-second.txt",
    "second.stub",
    "shared.txt",
];

#[test]
fn single_entry_file_set() {
    let t = trace(builder(0), &["main.stub"]);
    assert_eq!(rel_files(&t, t.files()), MAIN_FILES);
    assert_eq!(rel_files(&t, t.entry_files(0)), MAIN_FILES);
    let list = render(&t, Format::List);
    assert_eq!(list, MAIN_FILES.join("\n") + "\n");
    assert!(!list.contains("unreferenced"));
    assert!(!list.contains("README"));
    assert!(!list.contains("types.stub"));
}

#[test]
fn uncertainties_are_reported() {
    let t = trace(builder(0), &["main.stub"]);
    let kinds: Vec<(UncertaintyKind, String)> = t
        .uncertainties()
        .iter()
        .map(|(_, u, e)| (u.kind, e.map(|e| e.spec.to_string()).unwrap_or_default()))
        .collect();
    let has = |k: UncertaintyKind, spec: &str| kinds.iter().any(|(kk, s)| *kk == k && s == spec);
    assert!(
        has(UncertaintyKind::PartialSpecifier, "./locales/*.json"),
        "{kinds:?}"
    );
    assert!(
        has(UncertaintyKind::ComputedSpecifier, "<unknown>"),
        "{kinds:?}"
    );
    assert!(
        has(UncertaintyKind::UnresolvedSpecifier, "./missing.stub"),
        "{kinds:?}"
    );
    assert!(
        has(UncertaintyKind::UnresolvedSpecifier, "./also-missing"),
        "{kinds:?}"
    );
    assert!(
        has(UncertaintyKind::CaseMismatch, "./data/CONFIG.TXT"),
        "{kinds:?}"
    );
    assert_eq!(kinds.len(), 5, "{kinds:?}");

    // The glob edge records its matches.
    let glob = t
        .edges()
        .iter()
        .find(|e| matches!(e.to, Target::Glob { .. }))
        .unwrap();
    let Target::Glob {
        matches, truncated, ..
    } = &glob.to
    else {
        unreachable!()
    };
    assert_eq!(matches.len(), 3);
    assert!(!truncated);
}

#[test]
fn reasons_and_manifests() {
    let t = trace(builder(0), &["main.stub"]);
    let root = fixture();
    let reason = |rel: &str| t.reason(t.file_id(&root.join(rel)).unwrap()).unwrap();
    assert_eq!(
        reason("main.stub").kinds,
        ReasonKinds::INITIAL | ReasonKinds::DEPENDENCY
    );
    assert_eq!(reason("data/config.txt").kinds, ReasonKinds::ASSET);
    assert_eq!(
        reason("stub_modules/pkg/stub.json").kinds,
        ReasonKinds::MANIFEST
    );
    assert_eq!(reason("lib/util.stub").kinds, ReasonKinds::DEPENDENCY);
    let parents: Vec<_> = reason("main.stub")
        .parents
        .iter()
        .map(|&p| t.path(p).unwrap().to_path_buf())
        .collect();
    assert_eq!(parents, [root.join("lib/index.stub")]);
    assert!(
        t.edges()
            .iter()
            .any(|e| e.kind == RefKind::Manifest && e.spec.as_ref() == "pkg")
    );
}

#[test]
fn multi_entry_closures() {
    let t = trace(builder(0), &["main.stub", "second.stub"]);
    assert_eq!(rel_files(&t, t.entry_files(0)), MAIN_FILES);
    assert_eq!(rel_files(&t, t.entry_files(1)), SECOND_FILES);
    let mut union: Vec<&str> = MAIN_FILES.iter().chain(SECOND_FILES).copied().collect();
    union.sort();
    union.dedup();
    assert_eq!(rel_files(&t, t.files()), union);

    // Per-entry nft output.
    let mut out = Vec::new();
    let opts = OutputOptions {
        selection: Selection::Entry(1),
        ..Default::default()
    };
    output::write_nft(&t, &opts, &mut out).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["version"], 1);
    let files: Vec<&str> = v["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap())
        .collect();
    assert_eq!(files, SECOND_FILES);
    // data/link.txt is index 3 and points at data/config.txt.
    assert_eq!(v["symlinks"]["3"], "data/config.txt");
}

#[test]
fn symlinks_and_sniffing() {
    let t = trace(builder(0), &["second.stub"]);
    let root = fixture();
    let links: Vec<_> = t.symlinks().collect();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].0, root.join("data/link.txt"));
    assert!(links[0].1.ends_with("data/config.txt"));
    let tool = t.file_id(&root.join("bin/tool")).unwrap();
    assert!(t.is_parsed(tool));
    assert_eq!(t.language(tool).map(|l| l.0), Some("stub"));
    let target = t.file_id(&root.join("data/config.txt")).unwrap();
    assert!(
        t.reason(target)
            .unwrap()
            .kinds
            .contains(ReasonKinds::RESOLVE)
    );
}

#[test]
fn deterministic_across_thread_counts() {
    let entries = ["main.stub", "second.stub"];
    for format in [Format::Nft, Format::Json, Format::List] {
        let one = render(&trace(builder(1), &entries), format);
        for threads in [2, 8, 0] {
            for _ in 0..3 {
                let many = render(&trace(builder(threads), &entries), format);
                assert_eq!(one, many, "{format:?} differs with {threads} threads");
            }
        }
    }
}

#[test]
fn tracer_reuse_keeps_results_identical() {
    let tracer = builder(4).build().unwrap();
    let root = fixture();
    let a = tracer.trace(&[root.join("main.stub")]).unwrap();
    let dirs_after_first = tracer.fs_stats().dirs_read;
    let b = tracer.trace(&[root.join("main.stub")]).unwrap();
    assert_eq!(render(&a, Format::Json), render(&b, Format::Json));
    // Warm caches: no directory is read again.
    assert_eq!(tracer.fs_stats().dirs_read, dirs_after_first);
    // A different entry on the same tracer.
    let c = tracer.trace(&[root.join("second.stub")]).unwrap();
    assert_eq!(rel_files(&c, c.files()), SECOND_FILES);
}

#[test]
fn options_filter_output() {
    // Types included: types.stub and what it references.
    let t = trace(builder(0).include_types(true), &["main.stub"]);
    let files = rel_files(&t, t.files());
    assert!(files.contains(&"types.stub".to_string()));

    // Runtime only: the build-phase reference is dropped.
    let t = trace(builder(0).phase(PhaseFilter::Runtime), &["main.stub"]);
    let files = rel_files(&t, t.files());
    assert!(!files.contains(&"build-only.stub".to_string()));
    assert!(!files.contains(&"build-data.txt".to_string()));

    // Ignore globs.
    let t = trace(
        builder(0).ignore("*.json").ignore("assets/**"),
        &["main.stub"],
    );
    let files = rel_files(&t, t.files());
    assert!(
        files
            .iter()
            .all(|f| !f.ends_with(".json") && !f.starts_with("assets/"))
    );
    assert!(files.contains(&"main.stub".to_string()));

    // Glob limit.
    let t = trace(builder(0).max_glob(1), &["main.stub"]);
    assert!(
        t.uncertainties()
            .iter()
            .any(|(_, u, _)| u.kind == UncertaintyKind::PatternTooBroad)
    );
}

#[test]
fn why_chain() {
    let t = trace(builder(0), &["main.stub", "second.stub"]);
    let root = fixture();
    let chain = t
        .why(
            &root.join("main.stub"),
            &root.join("stub_modules/pkg/inner-data.txt"),
        )
        .unwrap();
    let hops: Vec<String> = chain
        .iter()
        .map(|s| t.relative(s.to).unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        hops,
        [
            "stub_modules/pkg/index.stub",
            "stub_modules/pkg/inner.stub",
            "stub_modules/pkg/inner-data.txt"
        ]
    );
    assert!(matches!(chain[0].via, WhyVia::Edge(e) if e.spec.as_ref() == "pkg"));

    let via_link = t
        .why(&root.join("second.stub"), &root.join("data/config.txt"))
        .unwrap();
    assert!(matches!(via_link.last().unwrap().via, WhyVia::Symlink));

    assert!(
        t.why(&root.join("second.stub"), &root.join("main.stub"))
            .is_none()
    );
    assert!(
        t.why(&root.join("main.stub"), &root.join("main.stub"))
            .unwrap()
            .is_empty()
    );
    assert!(
        t.why(&root.join("lib/util.stub"), &root.join("shared.txt"))
            .is_none()
    );
}

#[test]
fn json_schema_shape() {
    let t = trace(builder(0), &["main.stub"]);
    let v: serde_json::Value = serde_json::from_str(&render(&t, Format::Json)).unwrap();
    assert_eq!(v["version"], 1);
    assert_eq!(v["entries"][0]["path"], "main.stub");
    assert_eq!(v["stats"]["files"], MAIN_FILES.len());
    let main = v["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["path"] == "main.stub")
        .unwrap();
    assert_eq!(
        main["reasons"],
        serde_json::json!(["initial", "dependency"])
    );
    assert_eq!(main["lang"], "stub");
    let case = v["uncertainties"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["kind"] == "case_mismatch")
        .unwrap();
    assert_eq!(case["file"], "main.stub");
    assert_eq!(case["line"], 14);
    assert!(v["edges"].as_array().unwrap().len() > 10);
}

#[test]
fn missing_entry_is_an_error() {
    let tracer = builder(0).build().unwrap();
    let err = tracer.trace(&[fixture().join("nope.stub")]).unwrap_err();
    assert!(err.to_string().contains("entry not found"), "{err}");
    let err = tracer.trace(&[fixture().join("lib")]).unwrap_err();
    assert!(err.to_string().contains("not a file"), "{err}");
}

#[test]
fn unimplemented_options_fail_clearly() {
    let err = builder(0).hashes(true).build().unwrap_err();
    assert!(err.to_string().contains("not yet implemented"), "{err}");
}
