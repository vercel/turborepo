//! `cargo test` adapter tests on `fixtures/rust/{basic,registry}`.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]
#![cfg(feature = "rust")]

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use turborepo_trace_core::{DirCache, Platform};
use turborepo_trace_frameworks::{
    Framework, InputRole, SimpleResolver, Target, cargotest::CargoTest, framework,
};
use turborepo_trace_lang_rust::{CargoProject, manifest::TargetKind};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../turborepo-trace-testkit/fixtures/rust")
        .join(name)
        .canonicalize()
        .expect("fixture exists")
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .map(|r| r.to_string_lossy().into_owned())
        .unwrap_or_else(|_| p.display().to_string())
}

#[test]
fn registered_and_discovers_packages() {
    assert!(framework("cargo-test").is_some());
    let root = fixture("basic");
    let found: Vec<_> = CargoTest
        .discover(&root)
        .iter()
        .map(|p| rel(&root, p))
        .collect();
    assert_eq!(
        found,
        ["app/Cargo.toml", "excluded/Cargo.toml", "util/Cargo.toml"]
    );
    assert!(
        !CargoTest.is_config_file(&root.join("Cargo.toml")),
        "virtual manifest"
    );
    assert!(CargoTest.is_config_file(&root.join("app/Cargo.toml")));
    assert_eq!(
        CargoTest.config_for(&root.join("app/tests/integration.rs")),
        Some(root.join("app/Cargo.toml"))
    );
}

#[test]
fn package_inputs_and_discovery() {
    let root = fixture("basic");
    let fi = CargoTest.load(&root.join("app/Cargo.toml"));
    assert_eq!(fi.projects.len(), 1);
    let p = &fi.projects[0];
    assert_eq!(p.name, "app");
    let configs: Vec<_> = p
        .global
        .config_files
        .iter()
        .map(|f| rel(&root, f))
        .collect();
    for want in [
        "app/Cargo.toml",
        "Cargo.toml",
        "Cargo.lock",
        ".cargo/config.toml",
        "app/proto/api.proto",
        "app/build.rs",
    ] {
        assert!(configs.contains(&want.to_owned()), "{want} in {configs:?}");
    }
    assert_eq!(
        p.global
            .entries
            .iter()
            .map(|e| (e.role, e.target.clone()))
            .collect::<Vec<_>>(),
        [(
            InputRole::BuildScript,
            Target::Path(root.join("app/build.rs"))
        )]
    );
    // No library yet: adding `src/lib.rs` would link one into every integration
    // test.
    assert!(p.global.probes.contains(&root.join("app/src/lib.rs")));
    let tests: Vec<_> = p
        .discovery
        .enumerate()
        .iter()
        .map(|t| rel(&root, t))
        .collect();
    assert_eq!(tests, ["app/src/main.rs", "app/tests/integration.rs"]);
    let integration = root.join("app/tests/integration.rs");
    let ti = CargoTest.test_inputs(p, &integration, b"", &SimpleResolver::default());
    assert!(
        ti.entries.is_empty(),
        "app has no library: {:?}",
        ti.entries
    );
}

#[test]
fn build_script_without_rerun_if_changed_watches_the_package() {
    let d = std::env::temp_dir().join(format!("dt-cargotest-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let d = d.canonicalize().unwrap();
    std::fs::write(
        d.join("Cargo.toml"),
        "[package]\nname = \"b\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(d.join("src/lib.rs"), "").unwrap();
    std::fs::write(d.join("build.rs"), "fn main() {}\n").unwrap();
    let ci = CargoTest.crate_inputs(&d).unwrap();
    assert_eq!(ci.build_memberships.len(), 1);
    assert_eq!(ci.build_memberships[0].dir, d);
    assert_eq!(ci.build_memberships[0].pattern, "**");
    assert_eq!(ci.tests, [(TargetKind::Lib, d.join("src/lib.rs"))]);

    // A computed path is uncertain and also falls back to the whole package.
    std::fs::write(
        d.join("build.rs"),
        "fn main() { println!(\"cargo:rerun-if-changed={}\", p); }\n",
    )
    .unwrap();
    let ci = CargoTest.crate_inputs(&d).unwrap();
    assert_eq!(ci.uncertainties.len(), 1);
    assert_eq!(ci.build_memberships[0].pattern, "**");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn dev_dependency_identities_from_cargo_lock() {
    let root = fixture("registry");
    let model = CargoProject::new(
        Arc::new(DirCache::new()),
        Platform::parse("x86_64-unknown-linux-gnu").unwrap(),
        root.join("cargo-home"),
    );
    let ci = CargoTest.crate_inputs_with(&model, &root).unwrap();
    assert_eq!(ci.name, "regapp");
    let ids: Vec<_> = ci
        .dev_dependencies
        .iter()
        .map(|i| {
            format!(
                "{}@{} {}",
                i.name,
                i.version,
                i.checksum.as_deref().unwrap_or("-")
            )
        })
        .collect();
    assert_eq!(
        ids,
        ["serde_like@1.0.0 cccc000000000000000000000000000000000000000000000000000000000000"]
    );
    let kinds: Vec<_> = ci.tests.iter().map(|(k, r)| (*k, rel(&root, r))).collect();
    assert_eq!(
        kinds,
        [
            (TargetKind::Lib, "src/lib.rs".to_owned()),
            (TargetKind::Bin, "src/cli.rs".to_owned()),
        ]
    );
    // Identity of a file inside an opaque registry crate.
    let file = root
        .join("cargo-home/registry/src/index.crates.io-1949cf8c6b5b557f/dep-a-0.2.0/src/lib.rs");
    let id = model.identity(&file).unwrap();
    assert_eq!(
        (
            id.name.as_str(),
            id.version.as_str(),
            id.checksum.as_deref()
        ),
        (
            "dep-a",
            "0.2.0",
            Some("aaaa000000000000000000000000000000000000000000000000000000000000")
        )
    );
    let git = root
        .join("cargo-home/git/checkouts/gitws-0f1e2d3c4b5a6978/abcdef0/crates/gitdep/src/lib.rs");
    let id = model.identity(&git).unwrap();
    assert_eq!(id.name, "gitdep");
    assert!(
        id.source
            .unwrap()
            .ends_with("#abcdef0123456789abcdef0123456789abcdef01")
    );
}
