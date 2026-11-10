//! Hand-checked expectations for every fixture under `fixtures/lockfile`.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use turborepo_trace_lockfile::{
    DepKind, Error, Lockfile, LockfileFormat, LockfileKind, PackageFlags, Resolved, Target, detect,
    diff, parse, parse_path,
};

fn fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../turborepo-trace-testkit/fixtures/lockfile")
        .join(rel)
}

fn load(rel: &str) -> Lockfile {
    parse_path(fixture(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn ids(lock: &Lockfile) -> Vec<&str> {
    lock.packages().iter().map(|p| p.id.as_str()).collect()
}

fn resolved_id<'l>(r: Option<Resolved<'l>>) -> &'l str {
    match r {
        Some(Resolved::Package(p)) => &p.id,
        other => panic!("expected a package, got {other:?}"),
    }
}

fn closure_names(lock: &Lockfile, id: &str) -> Vec<String> {
    lock.closure(id)
        .iter()
        .map(|p| format!("{}@{}", p.name, p.version))
        .collect()
}

/// No package edge may point at an id that does not exist.
fn assert_no_dangling(lock: &Lockfile) {
    for p in lock.packages() {
        for e in &p.dependencies {
            if let Target::Package(id) = &e.target {
                assert!(lock.package(id).is_some(), "{} -> {} dangling", p.id, id);
            }
        }
    }
    for i in lock.importers() {
        for e in &i.dependencies {
            match &e.target {
                Target::Package(id) => {
                    assert!(lock.package(id).is_some(), "{} -> {id} dangling", i.path)
                }
                Target::Importer(p) => {
                    assert!(lock.importer(p).is_some(), "{} -> {p} dangling", i.path)
                }
                Target::Unresolved(_) => {}
            }
        }
    }
}

const PNPM_IDS: [&str; 13] = [
    "debug@4.3.4",
    "fsevents@2.3.3",
    "js-tokens@4.0.0",
    "left-pad@1.3.0(patch_hash=5c1e9ad2c1a4c4d0b2e0d3e95c1f7d0f8d34d1b0c2e5f2bcae7d4e0f9d1b2a3c)",
    "loose-envify@1.4.0",
    "ms@2.1.2",
    "ms@2.1.3",
    "react-dom@18.3.1(react@18.3.1)",
    "react@18.3.1",
    "scheduler@0.23.2",
    "string-width@4.2.3",
    "tarball-pkg@https://codeload.github.com/fx/tarball-pkg/tar.gz/abc123",
    "typescript@5.4.5",
];

#[test]
fn pnpm_v9() {
    let lock = load("pnpm/v9/pnpm-lock.yaml");
    assert_eq!(lock.format(), LockfileFormat::Pnpm);
    assert_eq!(lock.lockfile_version(), "9.0");
    assert_eq!(ids(&lock), PNPM_IDS);
    assert_no_dangling(&lock);
    let paths: Vec<&str> = lock.importers().iter().map(|i| i.path.as_str()).collect();
    assert_eq!(paths, [".", "packages/app", "packages/lib"]);

    // Importer resolution: link, alias, peer suffix, optional, dev.
    let app = lock.importer("packages/app").unwrap();
    assert_eq!(
        app.dependency("@fx/lib").unwrap().target,
        Target::Importer("packages/lib".into())
    );
    assert_eq!(
        resolved_id(lock.resolve("packages/app", "string-width-cjs")),
        "string-width@4.2.3"
    );
    assert_eq!(
        resolved_id(lock.resolve("packages/app", "react-dom")),
        "react-dom@18.3.1(react@18.3.1)"
    );
    assert_eq!(app.dependency("fsevents").unwrap().kind, DepKind::Optional);
    assert_eq!(
        app.dependency("react").unwrap().specifier.as_deref(),
        Some("catalog:")
    );
    assert_eq!(
        lock.importer(".")
            .unwrap()
            .dependency("typescript")
            .unwrap()
            .kind,
        DepKind::Dev
    );
    assert!(
        matches!(lock.resolve("./packages/app", "@fx/lib"), Some(Resolved::Importer(i)) if i.path == "packages/lib")
    );

    // Package fields.
    let fsevents = lock.package("fsevents@2.3.3").unwrap();
    assert_eq!(fsevents.platform.os, ["darwin"]);
    assert!(fsevents.flags.contains(PackageFlags::OPTIONAL));
    let lp = lock.resolve("packages/lib", "left-pad");
    let Some(Resolved::Package(lp)) = lp else {
        panic!()
    };
    assert_eq!(
        (lp.name.as_str(), lp.version.as_str()),
        ("left-pad", "1.3.0")
    );
    assert_eq!(
        lp.patch.as_deref(),
        Some("5c1e9ad2c1a4c4d0b2e0d3e95c1f7d0f8d34d1b0c2e5f2bcae7d4e0f9d1b2a3c")
    );
    assert_eq!(lp.integrity.as_deref(), Some("sha512-leftpad130=="));
    let tb = lock
        .package("tarball-pkg@https://codeload.github.com/fx/tarball-pkg/tar.gz/abc123")
        .unwrap();
    assert_eq!(
        (tb.name.as_str(), tb.version.as_str()),
        ("tarball-pkg", "0.1.0")
    );
    assert_eq!(
        tb.resolution.as_deref(),
        Some("https://codeload.github.com/fx/tarball-pkg/tar.gz/abc123")
    );
    assert_eq!(tb.integrity, None);

    // Closure and dependents.
    assert_eq!(
        closure_names(&lock, "react-dom@18.3.1(react@18.3.1)"),
        [
            "js-tokens@4.0.0",
            "loose-envify@1.4.0",
            "react-dom@18.3.1",
            "react@18.3.1",
            "scheduler@0.23.2"
        ]
    );
    let dependents: Vec<&str> = lock
        .dependents("loose-envify@1.4.0")
        .iter()
        .map(|p| p.id.as_str())
        .collect();
    assert_eq!(
        dependents,
        [
            "loose-envify@1.4.0",
            "react-dom@18.3.1(react@18.3.1)",
            "react@18.3.1",
            "scheduler@0.23.2"
        ]
    );
    let app_closure: Vec<&str> = lock
        .importer_closure("packages/app", false)
        .iter()
        .map(|p| p.id.as_str())
        .collect();
    assert!(
        app_closure.contains(&"ms@2.1.3"),
        "follows the workspace link into packages/lib"
    );
    assert!(app_closure.contains(&"ms@2.1.2"));
    assert!(!app_closure.contains(&"typescript@5.4.5"));

    // Metadata.
    let meta = lock.meta();
    assert_eq!(
        meta.overrides.get("ms@<2.1.2").map(String::as_str),
        Some("2.1.2")
    );
    assert_eq!(meta.catalogs["default"]["react"].version, "18.3.1");
    assert_eq!(
        meta.patched_dependencies["left-pad@1.3.0"],
        "5c1e9ad2c1a4c4d0b2e0d3e95c1f7d0f8d34d1b0c2e5f2bcae7d4e0f9d1b2a3c"
    );
}

#[test]
fn pnpm_v6_matches_v9() {
    let v6 = load("pnpm/v6/pnpm-lock.yaml");
    assert_eq!(v6.lockfile_version(), "6.0");
    assert_no_dangling(&v6);
    let mut expected: Vec<&str> = PNPM_IDS.to_vec();
    expected.retain(|id| !id.starts_with("tarball-pkg"));
    expected.insert(0, "@codeload.github.com/fx/tarball-pkg/tar.gz/abc123");
    assert_eq!(ids(&v6), expected);
    assert_eq!(
        resolved_id(v6.resolve("packages/app", "string-width-cjs")),
        "string-width@4.2.3"
    );
    assert!(
        v6.package("typescript@5.4.5")
            .unwrap()
            .flags
            .contains(PackageFlags::DEV)
    );
    assert!(
        v6.package("fsevents@2.3.3")
            .unwrap()
            .flags
            .contains(PackageFlags::OPTIONAL)
    );
    assert_eq!(v6.meta().patched_dependencies["left-pad@1.3.0"].len(), 64);

    // Same tree in both versions: ids differ only for the tarball package, and no
    // content identity or closure changed.
    let v9 = load("pnpm/v9/pnpm-lock.yaml");
    let d = diff(&v6, &v9);
    assert!(
        d.affected_packages().identity_changed.is_empty(),
        "{:?}",
        d.affected_packages()
    );
    assert!(
        d.affected_packages().closure_changed.is_empty(),
        "{:?}",
        d.affected_packages()
    );
    assert!(d.version_changes.is_empty());
    assert!(d.importers.is_empty(), "{:?}", d.importers);
}

#[test]
fn pnpm_v6_single_project() {
    let lock = load("pnpm/v6-single/pnpm-lock.yaml");
    assert_eq!(ids(&lock), ["left-pad@1.3.0", "ms@2.1.3"]);
    let root = lock.importer(".").unwrap();
    assert_eq!(root.dependency("ms").unwrap().kind, DepKind::Prod);
    assert_eq!(root.dependency("left-pad").unwrap().kind, DepKind::Dev);
}

#[test]
fn pnpm_env_document_is_skipped() {
    let lock = load("pnpm/v9-env-doc/pnpm-lock.yaml");
    assert_eq!(ids(&lock), ["ms@2.1.3"]);
    assert_eq!(resolved_id(lock.resolve(".", "ms")), "ms@2.1.3");
    assert!(
        lock.importer(".")
            .unwrap()
            .dependency("@fx/config")
            .is_none()
    );
}

#[test]
fn pnpm_unsupported_version() {
    let err = parse(LockfileKind::Pnpm, "lockfileVersion: 5.4\n").unwrap_err();
    assert!(matches!(err, Error::UnsupportedVersion { .. }), "{err}");
}

#[test]
fn npm_v3() {
    let lock = load("npm/v3/package-lock.json");
    assert_eq!(
        (lock.format(), lock.lockfile_version()),
        (LockfileFormat::Npm, "3")
    );
    assert_no_dangling(&lock);
    assert_eq!(
        ids(&lock),
        [
            "node_modules/debug",
            "node_modules/debug/node_modules/ms",
            "node_modules/fsevents",
            "node_modules/js-tokens",
            "node_modules/loose-envify",
            "node_modules/ms",
            "node_modules/react",
            "node_modules/react-dom",
            "node_modules/scheduler",
            "node_modules/string-width-cjs",
            "node_modules/typescript",
            "packages/lib/node_modules/left-pad",
        ]
    );
    let paths: Vec<&str> = lock.importers().iter().map(|i| i.path.as_str()).collect();
    assert_eq!(paths, [".", "packages/app", "packages/lib"]);
    // Nested resolution walks up from the dependent's own node_modules.
    assert_eq!(
        resolved_id(lock.resolve_from_package("node_modules/debug", "ms")),
        "node_modules/debug/node_modules/ms"
    );
    assert_eq!(
        lock.package("node_modules/debug/node_modules/ms")
            .unwrap()
            .version,
        "2.1.2"
    );
    assert_eq!(
        resolved_id(lock.resolve("packages/lib", "ms")),
        "node_modules/ms"
    );
    assert_eq!(
        resolved_id(lock.resolve("packages/lib", "left-pad")),
        "packages/lib/node_modules/left-pad"
    );
    assert!(
        matches!(lock.resolve("packages/app", "@fx/lib"), Some(Resolved::Importer(i)) if i.path == "packages/lib")
    );
    // Alias: folder name is the edge name, the package keeps its real name.
    let sw = lock.package("node_modules/string-width-cjs").unwrap();
    assert_eq!(
        (sw.name.as_str(), sw.version.as_str()),
        ("string-width", "4.2.3")
    );
    let react_dom = lock.package("node_modules/react-dom").unwrap();
    assert_eq!(react_dom.dependency("react").unwrap().kind, DepKind::Peer);
    assert_eq!(
        react_dom.dependency("react").unwrap().target,
        Target::Package("node_modules/react".into())
    );
    let fsevents = lock.package("node_modules/fsevents").unwrap();
    assert!(fsevents.flags.contains(PackageFlags::OPTIONAL));
    assert_eq!(fsevents.platform.os, ["darwin"]);
    assert!(
        lock.package("node_modules/typescript")
            .unwrap()
            .flags
            .contains(PackageFlags::DEV)
    );
    assert_eq!(
        lock.importer(".")
            .unwrap()
            .dependency("typescript")
            .unwrap()
            .kind,
        DepKind::Dev
    );
}

#[test]
fn npm_v2_reads_packages_and_ignores_legacy_tree() {
    let lock = load("npm/v2/package-lock.json");
    assert_eq!(lock.lockfile_version(), "2");
    assert_eq!(
        ids(&lock),
        [
            "node_modules/debug",
            "node_modules/debug/node_modules/ms",
            "node_modules/left-pad",
            "node_modules/ms"
        ]
    );
    assert_eq!(resolved_id(lock.resolve(".", "ms")), "node_modules/ms");
    assert_eq!(
        lock.importer(".")
            .unwrap()
            .dependency("left-pad")
            .unwrap()
            .kind,
        DepKind::Dev
    );
}

#[test]
fn npm_v1_is_unsupported() {
    let err = parse(
        LockfileKind::Npm,
        r#"{"lockfileVersion": 1, "dependencies": {}}"#,
    )
    .unwrap_err();
    assert!(
        matches!(err, Error::UnsupportedVersion { ref version, .. } if version == "1"),
        "{err}"
    );
}

#[test]
fn yarn_classic() {
    let mut lock = load("yarn/classic/yarn.lock");
    assert_eq!(lock.format(), LockfileFormat::YarnClassic);
    assert_no_dangling(&lock);
    assert_eq!(
        ids(&lock),
        [
            "chokidar-lite@1.0.0",
            "debug@4.3.4",
            "fsevents@2.3.3",
            "js-tokens@4.0.0",
            "left-pad@1.3.0",
            "loose-envify@1.4.0",
            "ms@2.1.2",
            "ms@2.1.3",
            "react-dom@18.3.1",
            "react@18.3.1",
            "scheduler@0.23.2",
            "string-width@4.2.3",
            "tarball-pkg@0.1.0",
            "typescript@5.4.5",
        ]
    );
    assert!(lock.importers().is_empty());
    let chokidar = lock.package("chokidar-lite@1.0.0").unwrap();
    assert_eq!(
        chokidar.dependency("fsevents").unwrap().kind,
        DepKind::Optional
    );
    assert_eq!(
        chokidar.dependency("fsevents").unwrap().target,
        Target::Package("fsevents@2.3.3".into())
    );
    assert_eq!(
        resolved_id(lock.resolve_from_package("debug@4.3.4", "ms")),
        "ms@2.1.2"
    );
    assert_eq!(
        lock.resolve_descriptor("react", "^18.3.0").unwrap().id,
        "react@18.3.1"
    );

    // Importers come from manifests.
    lock.add_manifest_importers(&[
        ("", r#"{"name": "fx-root", "devDependencies": {"typescript": "^5.4.0"}}"#),
        (
            "packages/app",
            r#"{"name": "@fx/app", "dependencies": {"@fx/lib": "*", "debug": "4.3.4", "react": "^18.3.0",
                "react-dom": "^18.3.1", "string-width-cjs": "npm:string-width@4.2.3"},
                "optionalDependencies": {"fsevents": "^2.3.3"}}"#,
        ),
        ("packages/lib", r#"{"name": "@fx/lib", "dependencies": {"left-pad": "1.3.0", "ms": "^2.1.3"}}"#),
    ])
    .unwrap();
    assert_no_dangling(&lock);
    assert_eq!(lock.importers().len(), 3);
    assert_eq!(
        resolved_id(lock.resolve("packages/app", "string-width-cjs")),
        "string-width@4.2.3"
    );
    assert!(
        matches!(lock.resolve("packages/app", "@fx/lib"), Some(Resolved::Importer(i)) if i.path == "packages/lib")
    );
    assert_eq!(
        resolved_id(lock.resolve(".", "typescript")),
        "typescript@5.4.5"
    );
    // Regression: the resolved importer-edge tables are rebuilt after attaching
    // manifests.
    assert!(diff(&lock, &lock).is_empty());
    let app_closure = lock.importer_closure("packages/app", false);
    assert!(
        app_closure.iter().any(|p| p.id == "ms@2.1.3"),
        "follows the link into packages/lib"
    );
}

// rustfmt's `format_strings` would change this item's string literals.
#[rustfmt::skip]
#[test]
fn yarn_classic_disambiguates_same_version_from_different_sources() {
    let src = "\
# yarn lockfile v1

foo@1.0.0:
  version \"1.0.0\"
  resolved \"https://registry.yarnpkg.com/foo/-/foo-1.0.0.tgz#aa\"
  integrity sha512-reg==

\"foo@git+https://example.com/foo.git#v1\":
  version \"1.0.0\"
  resolved \"git+https://example.com/foo.git#deadbeef\"
";
    let lock = parse(LockfileKind::YarnClassic, src).unwrap();
    assert_eq!(
        ids(&lock),
        [
            "foo@1.0.0#git+https://example.com/foo.git#deadbeef",
            "foo@1.0.0#sha512-reg=="
        ]
    );
}

#[test]
fn yarn_berry_v8() {
    let lock = load("yarn/berry-v8/yarn.lock");
    assert_eq!(
        (lock.format(), lock.lockfile_version()),
        (LockfileFormat::YarnBerry, "8")
    );
    assert_no_dangling(&lock);
    assert_eq!(lock.packages().len(), 13);
    let paths: Vec<&str> = lock.importers().iter().map(|i| i.path.as_str()).collect();
    assert_eq!(paths, [".", "packages/app", "packages/lib"]);
    assert_eq!(lock.importer(".").unwrap().name.as_deref(), Some("fx-root"));
    assert!(
        matches!(lock.resolve("packages/app", "@fx/lib"), Some(Resolved::Importer(i)) if i.path == "packages/lib")
    );
    assert_eq!(
        resolved_id(lock.resolve("packages/app", "string-width-cjs")),
        "string-width@npm:4.2.3"
    );
    assert_eq!(
        lock.importer("packages/app")
            .unwrap()
            .dependency("fsevents")
            .unwrap()
            .kind,
        DepKind::Optional
    );
    assert_eq!(
        resolved_id(lock.resolve_from_package("debug@npm:4.3.4", "ms")),
        "ms@npm:2.1.2"
    );
    let fsevents = lock.package("fsevents@npm:2.3.3").unwrap();
    assert_eq!(fsevents.platform.os, ["darwin"]);
    assert_eq!(fsevents.integrity.as_deref(), Some("10c0/fsevents233"));
    let Some(Resolved::Package(lp)) = lock.resolve("packages/lib", "left-pad") else {
        panic!()
    };
    assert_eq!(lp.patch.as_deref(), Some("4c3b5e"));
    assert_eq!(
        (lp.name.as_str(), lp.version.as_str()),
        ("left-pad", "1.3.0")
    );
    // Peers are resolved by yarn at link time, not in the lockfile.
    let rd = lock.package("react-dom@npm:18.3.1").unwrap();
    assert_eq!(
        rd.dependency("react").unwrap().target,
        Target::Unresolved("^18.3.1".into())
    );
}

#[test]
fn yarn_berry_v6_ranges_without_protocol() {
    let lock = load("yarn/berry-v6/yarn.lock");
    assert_eq!(lock.lockfile_version(), "6");
    assert_no_dangling(&lock);
    assert_eq!(ids(&lock), ["debug@npm:4.3.4", "ms@npm:2.1.2"]);
    assert_eq!(resolved_id(lock.resolve(".", "debug")), "debug@npm:4.3.4");
    assert_eq!(
        resolved_id(lock.resolve_from_package("debug@npm:4.3.4", "ms")),
        "ms@npm:2.1.2"
    );
}

#[test]
fn bun_v1() {
    let lock = load("bun/v1/bun.lock");
    assert_eq!(
        (lock.format(), lock.lockfile_version()),
        (LockfileFormat::Bun, "1")
    );
    assert_no_dangling(&lock);
    assert_eq!(
        ids(&lock),
        [
            "@fx/lib/left-pad",
            "debug",
            "debug/ms",
            "fsevents",
            "js-tokens",
            "loose-envify",
            "ms",
            "react",
            "react-dom",
            "scheduler",
            "string-width-cjs",
            "typescript",
        ]
    );
    let paths: Vec<&str> = lock.importers().iter().map(|i| i.path.as_str()).collect();
    assert_eq!(paths, [".", "packages/app", "packages/lib"]);
    assert_eq!(
        resolved_id(lock.resolve_from_package("debug", "ms")),
        "debug/ms"
    );
    assert_eq!(
        resolved_id(lock.resolve("packages/lib", "left-pad")),
        "@fx/lib/left-pad"
    );
    assert_eq!(resolved_id(lock.resolve("packages/lib", "ms")), "ms");
    assert!(
        matches!(lock.resolve("packages/app", "@fx/lib"), Some(Resolved::Importer(i)) if i.path == "packages/lib")
    );
    let sw = lock.package("string-width-cjs").unwrap();
    assert_eq!(
        (sw.name.as_str(), sw.version.as_str()),
        ("string-width", "4.2.3")
    );
    assert_eq!(lock.package("fsevents").unwrap().platform.os, ["darwin"]);
    // The unmet optional peer `supports-color` is dropped, the met peer kept.
    assert!(
        lock.package("debug")
            .unwrap()
            .dependency("supports-color")
            .is_none()
    );
    assert_eq!(
        lock.package("react-dom")
            .unwrap()
            .dependency("react")
            .unwrap()
            .kind,
        DepKind::Peer
    );
    assert_eq!(lock.meta().patched_dependencies.len(), 1);
}

#[test]
fn detection() {
    assert_eq!(detect("a/pnpm-lock.yaml", "").unwrap(), LockfileKind::Pnpm);
    assert_eq!(
        detect("npm-shrinkwrap.json", "").unwrap(),
        LockfileKind::Npm
    );
    assert_eq!(
        detect("yarn.lock", "# yarn lockfile v1\n").unwrap(),
        LockfileKind::YarnClassic
    );
    assert_eq!(
        detect("yarn.lock", "__metadata:\n  version: 8\n").unwrap(),
        LockfileKind::YarnBerry
    );
    assert_eq!(detect("bun.lock", "{}").unwrap(), LockfileKind::Bun);
    assert!(matches!(
        detect("bun.lockb", ""),
        Err(Error::Unsupported(_))
    ));
    assert_eq!(
        detect("old.yaml", "lockfileVersion: '9.0'\n").unwrap(),
        LockfileKind::Pnpm
    );
}

#[test]
fn equivalent_trees_across_formats() {
    // The same logical tree in every format: identical name@version sets (modulo
    // yarn classic's extra chokidar-lite and pnpm's tarball naming).
    let set = |lock: &Lockfile| -> std::collections::BTreeSet<String> {
        lock.packages()
            .iter()
            .filter(|p| p.name != "chokidar-lite" && p.name != "tarball-pkg")
            .map(|p| format!("{}@{}", p.name, p.version))
            .collect()
    };
    let pnpm = set(&load("pnpm/v9/pnpm-lock.yaml"));
    for rel in [
        "npm/v3/package-lock.json",
        "yarn/classic/yarn.lock",
        "bun/v1/bun.lock",
    ] {
        assert_eq!(set(&load(rel)), pnpm, "{rel}");
    }
    // Berry has both the unpatched and the patched left-pad.
    assert_eq!(set(&load("yarn/berry-v8/yarn.lock")), pnpm);
}
