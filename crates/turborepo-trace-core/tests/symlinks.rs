//! Symlinked path components through the whole tracer: pnpm
//! layouts, file links, entries given through links, and nft output.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    fs,
    path::{Path, PathBuf},
};

use turborepo_trace_core::{
    ExtractCx, FileId, LangId, Language, ProjectCx, RawRef, ReasonKinds, RefKind, RefSink,
    Resolution, ResolveCx, Resolver, Span, Specifier, Trace, Tracer, Uncertainty, UncertaintyKind,
    fs::normalize,
    os::symlink,
    output::{self, OutputOptions},
};

/// `.t` files: each line `<path>` is an import relative to the file (or, for
/// `@name`, of `node_modules/name/index.t` found by walking up). Lines starting
/// with `=` are manifests.
struct Lines;

impl Language for Lines {
    fn id(&self) -> LangId {
        LangId("t")
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["t"]
    }
    fn extract<'s>(&self, src: &'s [u8], _cx: &ExtractCx<'_>, out: &mut RefSink<'s>) {
        let mut pos = 0u32;
        for line in src.split(|&b| b == b'\n') {
            let start = pos;
            pos += line.len() as u32 + 1;
            let Ok(s) = std::str::from_utf8(line) else {
                continue;
            };
            if s.is_empty() {
                continue;
            }
            let (kind, s) = match s.strip_prefix('=') {
                Some(m) => (RefKind::Manifest, m),
                None => (RefKind::Import, s),
            };
            out.push(RawRef::new(
                kind,
                Specifier::literal(s),
                Span::new(start, start + line.len() as u32),
            ));
        }
    }
    fn resolver(&self, _project: &ProjectCx<'_>) -> Box<dyn Resolver> {
        Box::new(LinesResolver)
    }
}

struct LinesResolver;

impl Resolver for LinesResolver {
    fn resolve(&self, r: &RawRef<'_>, _from: FileId, cx: &ResolveCx<'_>) -> Resolution {
        let spec = r.spec.as_literal().unwrap_or_default();
        let path = if let Some(pkg) = spec.strip_prefix('@') {
            let mut dir = Some(cx.from_dir);
            let mut found = None;
            while let Some(d) = dir {
                let cand = d.join("node_modules").join(pkg).join("index.t");
                if cx.fs.is_file(&cand) {
                    found = Some(cand);
                    break;
                }
                dir = d.parent();
            }
            found
        } else {
            Some(normalize(&cx.from_dir.join(spec))).filter(|p| cx.fs.exists(p))
        };
        match path {
            Some(p) => Resolution::file(cx.intern(&p)),
            None => Resolution::unresolved(Uncertainty::new(
                UncertaintyKind::UnresolvedSpecifier,
                r.span,
                spec,
            )),
        }
    }
}

/// ```text
/// app/main.t                     -> @foo, ./link.t
/// app/real.t
/// app/link.t -> real.t
/// app/entry-link.t -> main.t
/// app/node_modules/foo -> .pnpm/foo@1.0.0/node_modules/foo
/// app/node_modules/.pnpm/foo@1.0.0/node_modules/foo/index.t   -> @bar, =./package.json
/// app/node_modules/.pnpm/foo@1.0.0/node_modules/foo/package.json
/// app/node_modules/.pnpm/foo@1.0.0/node_modules/bar -> ../../bar@2.0.0/node_modules/bar
/// app/node_modules/.pnpm/bar@2.0.0/node_modules/bar/index.t
/// ```
fn pnpm_app() -> (tempfile::TempDir, PathBuf) {
    let t = tempfile::tempdir().unwrap();
    let app = normalize(t.path()).join("app");
    let foo_dir = app.join("node_modules/.pnpm/foo@1.0.0/node_modules/foo");
    let bar_dir = app.join("node_modules/.pnpm/bar@2.0.0/node_modules/bar");
    fs::create_dir_all(&foo_dir).unwrap();
    fs::create_dir_all(&bar_dir).unwrap();
    fs::write(app.join("main.t"), "@foo\n./link.t\n").unwrap();
    fs::write(app.join("real.t"), "").unwrap();
    symlink("real.t", app.join("link.t")).unwrap();
    symlink("main.t", app.join("entry-link.t")).unwrap();
    fs::write(foo_dir.join("index.t"), "@bar\n=./package.json\n").unwrap();
    fs::write(foo_dir.join("package.json"), "{}").unwrap();
    fs::write(bar_dir.join("index.t"), "").unwrap();
    symlink(
        ".pnpm/foo@1.0.0/node_modules/foo",
        app.join("node_modules/foo"),
    )
    .unwrap();
    symlink(
        "../../bar@2.0.0/node_modules/bar",
        app.join("node_modules/.pnpm/foo@1.0.0/node_modules/bar"),
    )
    .unwrap();
    (t, app)
}

fn trace(app: &Path, threads: usize, entries: &[&str]) -> Trace {
    let entries: Vec<PathBuf> = entries.iter().map(|e| app.join(e)).collect();
    Tracer::builder()
        .base(app)
        .threads(threads)
        .language(Lines)
        .build()
        .unwrap()
        .trace(&entries)
        .unwrap()
}

fn rel(t: &Trace, p: &Path) -> String {
    t.relative(p).unwrap().to_string_lossy().into_owned()
}

#[test]
fn pnpm_layout_files_symlinks_and_reasons() {
    let (_t, app) = pnpm_app();
    let t = trace(&app, 0, &["main.t"]);
    let files: Vec<String> = t.files().map(|p| rel(&t, p)).collect();
    assert_eq!(
        files,
        [
            "link.t",
            "main.t",
            "node_modules/.pnpm/bar@2.0.0/node_modules/bar/index.t",
            "node_modules/.pnpm/foo@1.0.0/node_modules/bar",
            "node_modules/.pnpm/foo@1.0.0/node_modules/foo/index.t",
            "node_modules/.pnpm/foo@1.0.0/node_modules/foo/package.json",
            "node_modules/foo",
            "real.t",
        ]
    );
    let links: Vec<(String, String)> = t
        .symlinks()
        .map(|(l, t2)| (rel(&t, l), rel(&t, t2)))
        .collect();
    assert_eq!(
        links,
        [
            ("link.t".into(), "real.t".into()),
            (
                "node_modules/.pnpm/foo@1.0.0/node_modules/bar".into(),
                "node_modules/.pnpm/bar@2.0.0/node_modules/bar".into()
            ),
            (
                "node_modules/foo".into(),
                "node_modules/.pnpm/foo@1.0.0/node_modules/foo".into()
            ),
        ]
    );

    // Real files are parsed; links are not.
    let id = |p: &str| t.file_id(&app.join(p)).unwrap();
    assert!(t.is_parsed(id("node_modules/.pnpm/foo@1.0.0/node_modules/foo/index.t")));
    assert!(t.is_parsed(id("node_modules/.pnpm/bar@2.0.0/node_modules/bar/index.t")));
    assert!(!t.is_parsed(id("node_modules/foo")));

    // Links carry `resolve` with the referencing file as parent.
    let r = t.reason(id("node_modules/foo")).unwrap();
    assert_eq!(r.kinds, ReasonKinds::RESOLVE);
    assert_eq!(r.parents, [id("main.t")]);
    let r = t
        .reason(id("node_modules/.pnpm/foo@1.0.0/node_modules/foo/index.t"))
        .unwrap();
    assert_eq!(r.kinds, ReasonKinds::DEPENDENCY);
    // Manifest reached through the package's real location.
    let r = t
        .reason(id(
            "node_modules/.pnpm/foo@1.0.0/node_modules/foo/package.json"
        ))
        .unwrap();
    assert_eq!(r.kinds, ReasonKinds::MANIFEST);

    // Edges point at real paths.
    let to_foo = t.edges().iter().find(|e| &*e.spec == "@foo").unwrap();
    assert_eq!(
        to_foo.to,
        turborepo_trace_core::Target::File(id(
            "node_modules/.pnpm/foo@1.0.0/node_modules/foo/index.t"
        ))
    );

    // nft: link paths are files, and `symlinks` maps their indexes to targets.
    let mut out = Vec::new();
    output::write_nft(&t, &OutputOptions::default(), &mut out).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["files"].as_array().unwrap().len(), 8);
    assert_eq!(v["symlinks"]["0"], "real.t");
    assert_eq!(
        v["symlinks"]["3"],
        "node_modules/.pnpm/bar@2.0.0/node_modules/bar"
    );
    assert_eq!(
        v["symlinks"]["6"],
        "node_modules/.pnpm/foo@1.0.0/node_modules/foo"
    );

    // `why` through a file link shows the link hop.
    let chain = t.why(&app.join("main.t"), &app.join("real.t")).unwrap();
    assert_eq!(chain.len(), 2);
    assert_eq!(rel(&t, chain[0].to), "link.t");
}

#[test]
fn entry_through_symlink_is_traced_at_real_path() {
    let (_t, app) = pnpm_app();
    let t = trace(&app, 0, &["entry-link.t"]);
    let entries: Vec<String> = t.entries().map(|p| rel(&t, p)).collect();
    assert_eq!(entries, ["main.t"]);
    let files: Vec<String> = t.entry_files(0).map(|p| rel(&t, p)).collect();
    assert!(files.contains(&"entry-link.t".to_string()), "{files:?}");
    assert!(files.contains(&"node_modules/foo".to_string()), "{files:?}");
    let link = t.file_id(&app.join("entry-link.t")).unwrap();
    assert!(t.reason(link).unwrap().kinds.contains(ReasonKinds::RESOLVE));
}

#[test]
fn symlink_output_is_deterministic() {
    let (_t, app) = pnpm_app();
    let render = |threads| {
        let mut out = Vec::new();
        output::write_json(&trace(&app, threads, &["main.t", "entry-link.t"]), &mut out).unwrap();
        String::from_utf8(out).unwrap()
    };
    let one = render(1);
    for threads in [2, 8] {
        assert_eq!(one, render(threads));
    }
}
