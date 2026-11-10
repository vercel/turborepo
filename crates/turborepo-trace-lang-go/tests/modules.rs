//! Module resolution edge cases on synthetic layouts: the MVS fallback for old
//! `go.mod` files, vendor modes, `go.work` replacements and standard library
//! detection.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{path::Path, sync::Arc};

use turborepo_trace_core::{DirCache, ProjectCx, TraceOptions};
use turborepo_trace_lang_go::{GoOptions, GoResolver, GoRoot, VendorMode, modules::PkgLoc};

fn write(root: &Path, files: &[(&str, &str)]) {
    for (f, c) in files {
        let p = root.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, c).unwrap();
    }
}

/// A resolver for trace base `base`; the module cache is `<base>/mod` unless
/// `opts` names one.
fn resolver(base: &Path, opts: GoOptions) -> (GoResolver, Arc<DirCache>) {
    let fs = Arc::new(DirCache::new());
    let options = TraceOptions {
        base: base.to_path_buf(),
        cwd: base.to_path_buf(),
        target: Some("linux/amd64".into()),
        ..TraceOptions::default()
    };
    let r = GoResolver::new(
        &ProjectCx {
            options: &options,
            fs: &fs,
        },
        &GoOptions {
            modcache: opts.modcache.clone().or_else(|| Some(base.join("mod"))),
            goroot: GoRoot::None,
            workspace: opts.workspace.or(Some(true)),
            ..opts
        },
    );
    (r, fs)
}

fn dir_of(loc: &PkgLoc) -> Option<&Path> {
    match loc {
        PkgLoc::Dir { dir, .. } => Some(dir),
        _ => None,
    }
}

#[test]
fn mvs_fallback_selects_the_highest_required_version() {
    let t = tempfile::tempdir().unwrap();
    let r = t.path();
    write(
        r,
        &[
            // Pre-1.17 go.mod: b is only required through a.
            (
                "app/go.mod",
                "module example.com/app\n\ngo 1.16\n\nrequire example.com/a v1.0.0\n",
            ),
            ("app/main.go", "package main\n"),
            ("mod/example.com/a@v1.0.0/a.go", "package a\n"),
            (
                "mod/cache/download/example.com/a/@v/v1.0.0.mod",
                "module example.com/a\n\nrequire (\n\texample.com/b v1.1.0\n\texample.com/c \
                 v0.1.0\n)\n",
            ),
            (
                "mod/cache/download/example.com/c/@v/v0.1.0.mod",
                "module example.com/c\n\nrequire example.com/b v1.2.0\n",
            ),
            ("mod/example.com/b@v1.1.0/pkg/b.go", "package pkg\n"),
            ("mod/example.com/b@v1.2.0/pkg/b.go", "package pkg\n"),
        ],
    );
    let (res, fs) = resolver(
        &r.join("app"),
        GoOptions {
            modcache: Some(r.join("mod")),
            ..GoOptions::default()
        },
    );
    let loc = res.locate(&fs, &r.join("app"), "example.com/b/pkg");
    assert_eq!(
        dir_of(&loc),
        Some(r.join("mod/example.com/b@v1.2.0/pkg").as_path()),
        "{loc:?}"
    );
    match &*loc {
        PkgLoc::Dir { module, .. } => assert_eq!(
            module.as_ref().unwrap(),
            &("example.com/b".to_string(), "v1.2.0".to_string())
        ),
        _ => unreachable!(),
    }
    // A direct requirement resolves without MVS.
    let loc = res.locate(&fs, &r.join("app"), "example.com/a");
    assert_eq!(
        dir_of(&loc),
        Some(r.join("mod/example.com/a@v1.0.0").as_path())
    );
    // Unknown modules are reported.
    assert!(matches!(
        &*res.locate(&fs, &r.join("app"), "example.com/nope"),
        PkgLoc::Missing(_)
    ));
    // Dotless paths that are not in std are not modules.
    let loc = res.locate(&fs, &r.join("app"), "notstd/pkg");
    assert!(
        matches!(&*loc, PkgLoc::Missing(m) if m.contains("not in std")),
        "{loc:?}"
    );
    assert_eq!(*res.locate(&fs, &r.join("app"), "net/http"), PkgLoc::Std);
}

#[test]
fn vendor_modes() {
    let t = tempfile::tempdir().unwrap();
    let r = t.path();
    write(
        r,
        &[
            (
                "go.mod",
                "module example.com/v\n\ngo 1.21\n\nrequire example.com/d v1.0.0\n",
            ),
            (
                "vendor/modules.txt",
                "# example.com/d v1.0.0\n## explicit\nexample.com/d\n",
            ),
            ("vendor/example.com/d/d.go", "package d\n"),
            ("mod/example.com/d@v1.0.0/d.go", "package d\n"),
        ],
    );
    let (res, fs) = resolver(r, GoOptions::default());
    assert_eq!(
        dir_of(&res.locate(&fs, r, "example.com/d")),
        Some(r.join("vendor/example.com/d").as_path())
    );
    let (res, fs) = resolver(
        r,
        GoOptions {
            vendor: VendorMode::Off,
            ..GoOptions::default()
        },
    );
    assert_eq!(
        dir_of(&res.locate(&fs, r, "example.com/d")),
        Some(r.join("mod/example.com/d@v1.0.0").as_path())
    );
    // go < 1.14 does not vendor automatically.
    write(
        r,
        &[(
            "go.mod",
            "module example.com/v\n\ngo 1.13\n\nrequire example.com/d v1.0.0\n",
        )],
    );
    let (res, fs) = resolver(r, GoOptions::default());
    assert_eq!(
        dir_of(&res.locate(&fs, r, "example.com/d")),
        Some(r.join("mod/example.com/d@v1.0.0").as_path())
    );
}

#[test]
fn workspace_replacements_win() {
    let t = tempfile::tempdir().unwrap();
    let r = t.path();
    write(
        r,
        &[
            (
                "go.work",
                "go 1.22\n\nuse ./app\n\nreplace example.com/x => ./forks/x\n",
            ),
            (
                "app/go.mod",
                "module example.com/app\n\ngo 1.22\n\nrequire example.com/x v1.0.0\n\nreplace \
                 example.com/x => ../other\n",
            ),
            ("app/main.go", "package main\n"),
            ("forks/x/go.mod", "module example.com/x\n"),
            ("forks/x/x.go", "package x\n"),
            ("other/x.go", "package x\n"),
        ],
    );
    let (res, fs) = resolver(r, GoOptions::default());
    let loc = res.locate(&fs, &r.join("app"), "example.com/x");
    assert_eq!(dir_of(&loc), Some(r.join("forks/x").as_path()), "{loc:?}");
    // Without workspace mode the module's own replacement applies.
    let (res, fs) = resolver(
        &r.join("app"),
        GoOptions {
            workspace: Some(false),
            ..GoOptions::default()
        },
    );
    let loc = res.locate(&fs, &r.join("app"), "example.com/x");
    assert_eq!(dir_of(&loc), Some(r.join("other").as_path()), "{loc:?}");
}

/// The modules a not-downloaded import may come from, as `module@version`.
fn candidates(loc: &PkgLoc) -> Vec<String> {
    match loc {
        PkgLoc::NotDownloaded(ms) => ms.iter().map(|(m, v, _)| format!("{m}@{v}")).collect(),
        other => panic!("not NotDownloaded: {other:?}"),
    }
}

#[test]
fn a_required_module_that_was_never_downloaded() {
    let t = tempfile::tempdir().unwrap();
    let r = t.path();
    write(
        r,
        &[
            (
                "go.mod",
                "module example.com/app\n\ngo 1.22\n\nrequire (\n\texample.com/a \
                 v1.0.0\n\texample.com/b v1.1.0\n\texample.com/nest \
                 v0.5.0\n\texample.com/nest/deep v0.2.0\n\texample.com/Upper/c \
                 v1.0.0\n\texample.com/old v1.0.0\n\texample.com/loc v0.0.0\n)\n\nreplace \
                 example.com/old => example.com/fork v2.0.0\n\nreplace example.com/loc => ./loc\n",
            ),
            ("main.go", "package main\n"),
            // Downloaded: a (without the package `missing`), and the outer nested module.
            ("mod/example.com/a@v1.0.0/a.go", "package a\n"),
            ("mod/example.com/nest@v0.5.0/nest.go", "package nest\n"),
            ("loc/go.mod", "module example.com/loc\n"),
            ("loc/loc.go", "package loc\n"),
        ],
    );
    let (res, fs) = resolver(r, GoOptions::default());
    // Not in the module cache: the requirement names the module version.
    let loc = res.locate(&fs, r, "example.com/b/pkg/sub");
    assert_eq!(candidates(&loc), ["example.com/b@v1.1.0"]);
    match &*loc {
        PkgLoc::NotDownloaded(ms) => {
            assert_eq!(ms[0].2, r.join("mod/example.com/b@v1.1.0"));
        }
        _ => unreachable!(),
    }
    // The cache directory of a module with upper-case letters is escaped; the
    // identity is not.
    let loc = res.locate(&fs, r, "example.com/Upper/c");
    assert_eq!(candidates(&loc), ["example.com/Upper/c@v1.0.0"]);
    match &*loc {
        PkgLoc::NotDownloaded(ms) => {
            assert_eq!(ms[0].2, r.join("mod/example.com/!upper/c@v1.0.0"));
        }
        _ => unreachable!(),
    }
    // Below two nested modules: the inner one is not downloaded and may hold the
    // package; the outer one is on disk and does not.
    let loc = res.locate(&fs, r, "example.com/nest/deep/x");
    assert_eq!(candidates(&loc), ["example.com/nest/deep@v0.2.0"]);
    // A module replaced by another module version is that version.
    let loc = res.locate(&fs, r, "example.com/old/pkg");
    assert_eq!(candidates(&loc), ["example.com/fork@v2.0.0"]);
    // A downloaded module that lacks the package does not provide it.
    assert!(matches!(
        &*res.locate(&fs, r, "example.com/a/missing"),
        PkgLoc::Missing(_)
    ));
    // Neither does a local replacement (first-party code that is simply not there).
    assert!(matches!(
        &*res.locate(&fs, r, "example.com/loc/missing"),
        PkgLoc::Missing(_)
    ));
    // No requirement at all.
    assert!(matches!(
        &*res.locate(&fs, r, "example.com/unknown/pkg"),
        PkgLoc::Missing(_)
    ));
    // What is on disk still resolves to files.
    assert_eq!(
        dir_of(&res.locate(&fs, r, "example.com/a")),
        Some(r.join("mod/example.com/a@v1.0.0").as_path())
    );
}
