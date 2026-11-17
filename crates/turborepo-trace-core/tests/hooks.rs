//! Resolver hooks through the whole tracer: [`Resolution::none`], [`ExtraEdge`]
//! flags and phase, environment roots (no per-file `outside_base`), ignored
//! pseudo-filesystems, `parsed` for leaves reached by a following kind, and the
//! JSON/NDJSON/nft outputs.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    fs,
    path::{Path, PathBuf},
};

use turborepo_trace_core::{
    EnvRoot, ExtraEdge, ExtractCx, FileId, LangId, Language, Phase, PhaseFilter, ProjectCx, RawRef,
    RefFlags, RefKind, RefSink, Resolution, ResolveCx, Resolver, Span, Specifier, Target, Trace,
    Tracer, UncertaintyKind,
    fs::normalize,
    output::{self, OutputOptions},
};

/// `.h` files, one reference per line:
/// - `path`: import relative to the file;
/// - `!name`: an attribute, no file edge ([`Resolution::none`]);
/// - `^/abs/path`: an absolute runtime read;
/// - `%path`: import plus a conditional extra edge to `path.alt` and a
///   build-phase manifest `build.cfg` next to the file.
struct H {
    env: PathBuf,
}

impl Language for H {
    fn id(&self) -> LangId {
        LangId("h")
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["h"]
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
            let kind = if s.starts_with('^') {
                RefKind::FsRead
            } else {
                RefKind::Import
            };
            out.push(RawRef::new(
                kind,
                Specifier::literal(s),
                Span::new(start, start + line.len() as u32),
            ));
        }
    }
    fn resolver(&self, _project: &ProjectCx<'_>) -> Box<dyn Resolver> {
        Box::new(HResolver {
            env: self.env.clone(),
        })
    }
}

struct HResolver {
    env: PathBuf,
}

impl Resolver for HResolver {
    fn resolve(&self, r: &RawRef<'_>, _from: FileId, cx: &ResolveCx<'_>) -> Resolution {
        let spec = r.spec.as_literal().unwrap_or_default();
        if let Some(sub) = spec.strip_prefix('!') {
            // Probe a submodule candidate, then report "no edge, by design".
            let _ = cx.fs.is_file(&cx.from_dir.join(sub));
            return Resolution::none();
        }
        if let Some(abs) = spec.strip_prefix('^') {
            return Resolution::file(cx.intern(Path::new(abs)));
        }
        if let Some(p) = spec.strip_prefix('%') {
            let main = normalize(&cx.from_dir.join(p));
            let alt = normalize(&cx.from_dir.join(format!("{p}.alt")));
            let cfg = cx.from_dir.join("build.cfg");
            return Resolution::file(cx.intern(&main))
                .with_extra(
                    ExtraEdge::new(RefKind::Import, Target::File(cx.intern(&alt)), "alt")
                        .with_flags(RefFlags::CONDITIONAL),
                )
                .with_extra(
                    ExtraEdge::new(RefKind::Manifest, Target::File(cx.intern(&cfg)), "cfg")
                        .with_phase(Phase::Build),
                );
        }
        Resolution::file(cx.intern(&normalize(&cx.from_dir.join(spec))))
    }

    fn environment_roots(&self) -> Vec<EnvRoot> {
        vec![EnvRoot::new("h:env", self.env.clone())]
    }
}

struct Fixture {
    _t: tempfile::TempDir,
    app: PathBuf,
}

/// ```text
/// app/main.h      -> ./lib.h, !attr, ^/proc/self/status, ../env/pkg/a.h, ../other/x.h,
///                    %cond.h, native.c
/// app/lib.h, app/cond.h, app/cond.h.alt, app/build.cfg, app/native.c
/// env/pkg/a.h     -> ./b.h        (an environment root outside the base)
/// env/pkg/b.h
/// other/x.h                        (outside the base, not a root)
/// ```
fn fixture() -> Fixture {
    let t = tempfile::tempdir().unwrap();
    let base = normalize(&fs::canonicalize(t.path()).unwrap());
    let app = base.join("app");
    fs::create_dir_all(&app).unwrap();
    fs::create_dir_all(base.join("env/pkg")).unwrap();
    fs::create_dir_all(base.join("other")).unwrap();
    fs::write(
        app.join("main.h"),
        "./lib.h\n!attr\n^/proc/self/status\n../env/pkg/a.h\n../other/x.h\n%cond.h\nnative.c\n",
    )
    .unwrap();
    for f in ["lib.h", "cond.h", "cond.h.alt", "build.cfg", "native.c"] {
        fs::write(app.join(f), "").unwrap();
    }
    fs::write(base.join("env/pkg/a.h"), "./b.h\n").unwrap();
    fs::write(base.join("env/pkg/b.h"), "").unwrap();
    fs::write(base.join("other/x.h"), "").unwrap();
    Fixture { _t: t, app }
}

fn trace(f: &Fixture, phase: PhaseFilter) -> Trace {
    let env = f.app.parent().unwrap().join("env");
    let tracer = Tracer::builder()
        .base(&f.app)
        .cwd(&f.app)
        .phase(phase)
        .threads(2)
        .language(H { env })
        .build()
        .unwrap();
    tracer.trace(&[f.app.join("main.h")]).unwrap()
}

fn rel(t: &Trace, p: &Path) -> String {
    match t.relative(p) {
        Some(r) => r.display().to_string(),
        None => p.display().to_string(),
    }
}

#[test]
fn hooks_through_the_tracer() {
    let f = fixture();
    let t = trace(&f, PhaseFilter::All);
    let env = f.app.parent().unwrap().join("env");
    let files: Vec<String> = t.files().map(|p| rel(&t, p)).collect();

    // `/proc` is never traced; `!attr` has no edge at all.
    assert!(!files.iter().any(|p| p.starts_with("/proc")), "{files:?}");
    assert!(!t.edges().iter().any(|e| &*e.spec == "!attr"));
    assert!(
        !t.uncertainties()
            .iter()
            .any(|(_, u, _)| u.detail.contains("attr"))
    );

    // Extra edge flags and phase.
    let alt = t.edges().iter().find(|e| &*e.spec == "alt").unwrap();
    assert!(alt.flags.contains(RefFlags::CONDITIONAL));
    assert!(alt.uncertainty.is_none());
    assert_eq!(alt.phase, Phase::Runtime);
    let cfg = t.edges().iter().find(|e| &*e.spec == "cfg").unwrap();
    assert_eq!(cfg.phase, Phase::Build);

    // A `.c` file reached by an import has no language: a leaf, not parsed.
    let c = t.file_id(&f.app.join("native.c")).unwrap();
    assert!(!t.is_parsed(c));
    assert!(t.is_parsed(t.file_id(&f.app.join("lib.h")).unwrap()));

    // Environment root files: no per-file outside_base record, one root with 2
    // files; the unexpected outside file is still reported.
    assert_eq!(t.environment_roots().len(), 1);
    assert_eq!(&*t.environment_roots()[0].0.name, "h:env");
    assert_eq!(t.environment_roots()[0].1, 2);
    let outside: Vec<String> = t
        .uncertainties()
        .iter()
        .filter(|(_, u, _)| u.kind == UncertaintyKind::OutsideBase)
        .map(|(_, u, _)| u.detail.to_string())
        .collect();
    assert_eq!(outside.len(), 1, "{outside:?}");
    assert!(outside[0].contains("other/x.h"), "{outside:?}");
    let a = t.file_id(&env.join("pkg/a.h")).unwrap();
    assert_eq!(t.environment_root(a).map(|r| &*r.name), Some("h:env"));

    // nft: omitted, reported per root.
    let mut out = Vec::new();
    let report = output::write_nft(&t, &OutputOptions::default(), &mut out).unwrap();
    assert_eq!(report.outside_base.len(), 1);
    assert_eq!(
        report.environment_roots,
        [("h:env".to_string(), env.clone(), 2)]
    );

    // json: absolute path, `root`, no `outside_base`, top-level `roots`.
    let mut out = Vec::new();
    output::write_json(&t, &mut out).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["version"], 1);
    assert_eq!(v["roots"]["h:env"], env.display().to_string());
    let fa = v["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["path"] == env.join("pkg/a.h").display().to_string())
        .unwrap();
    assert_eq!(fa["root"], "h:env");
    assert!(fa.get("outside_base").is_none());

    // The build-phase manifest is dropped with --phase runtime.
    let rt = trace(&f, PhaseFilter::Runtime);
    assert!(!rt.edges().iter().any(|e| &*e.spec == "cfg"));
    assert!(rt.edges().iter().any(|e| &*e.spec == "alt"));
}

#[test]
fn ndjson_matches_json() {
    let f = fixture();
    let t = trace(&f, PhaseFilter::All);
    let mut nd = Vec::new();
    output::write_ndjson(&t, &mut nd).unwrap();
    let lines: Vec<serde_json::Value> = std::str::from_utf8(&nd)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let mut js = Vec::new();
    output::write_json(&t, &mut js).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&js).unwrap();
    let files: Vec<&serde_json::Value> = lines.iter().filter(|l| l["type"] == "file").collect();
    assert_eq!(files.len(), v["files"].as_array().unwrap().len());
    for (nf, jf) in files.iter().zip(v["files"].as_array().unwrap()) {
        assert_eq!(nf["path"], jf["path"]);
        assert_eq!(nf["parsed"], jf["parsed"]);
    }
    let edges = lines.iter().filter(|l| l["type"] == "edge").count();
    assert_eq!(edges, v["edges"].as_array().unwrap().len());
    assert_eq!(lines[0]["roots"], v["roots"]);
    let entry = lines.iter().find(|l| l["type"] == "entry").unwrap();
    let paths: Vec<&serde_json::Value> = entry["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| &files[i.as_u64().unwrap() as usize]["path"])
        .collect();
    let want: Vec<&serde_json::Value> = v["entries"][0]["files"]
        .as_array()
        .unwrap()
        .iter()
        .collect();
    assert_eq!(paths, want);
}
