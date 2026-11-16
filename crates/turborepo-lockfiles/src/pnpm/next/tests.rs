use std::path::Path;

use pretty_assertions::assert_eq;

use super::super::{Format, PnpmLockfile};
use crate::Lockfile;

/// The v13 lockfile's restored model next to its v9 twin's parsed model,
/// with what v13 does not carry (`deprecated`) and the format markers
/// normalized away.
fn normalized_pair(v9: &[u8], v13: &[u8]) -> (PnpmLockfile, PnpmLockfile) {
    let mut v9 = PnpmLockfile::from_bytes(v9).expect("v9 twin parses");
    let mut v13 = PnpmLockfile::from_bytes(v13).expect("v13 parses");
    assert!(matches!(v13.format, Format::Next(_)));
    for package in v9
        .packages
        .iter_mut()
        .flat_map(|packages| packages.values_mut())
    {
        package.other.remove("deprecated");
    }
    // pnpm's reader merges the integrity into a git-hosted tarball resolution,
    // and its v9 writer keeps or drops it depending on the input.
    for package in [&mut v9, &mut v13]
        .into_iter()
        .flat_map(|lockfile| lockfile.packages.iter_mut().flat_map(|p| p.values_mut()))
    {
        if package.resolution.other.contains_key("gitHosted") {
            package.resolution.integrity = None;
        }
    }
    v13.lockfile_version = v9.lockfile_version.clone();
    v13.format = Format::Suffixed;
    v9.dependency_index = Default::default();
    v13.dependency_index = Default::default();
    (v9, v13)
}

/// Paired oracle: pnpm 12.20.0 locked the same graph as v9 and as v13, so the
/// v13 restore must equal the v9 parse, and every query turbo makes must
/// agree.
pub(super) fn assert_paired(v9_bytes: &[u8], v13_bytes: &[u8], label: &str) {
    let (v9, mut v13) = normalized_pair(v9_bytes, v13_bytes);
    if let Some((_, reason)) = KNOWN_CATALOG_DIVERGENCE
        .iter()
        .find(|(name, _)| label.ends_with(name))
    {
        assert_ne!(
            v13.catalogs, v9.catalogs,
            "{label}: {reason} no longer holds"
        );
        v13.catalogs.clone_from(&v9.catalogs);
    }
    assert_eq!(v13.importers, v9.importers, "{label}: importers");
    assert_eq!(v13.snapshots, v9.snapshots, "{label}: snapshots");
    assert_eq!(v13.packages, v9.packages, "{label}: packages");
    assert_eq!(v13.settings, v9.settings, "{label}: settings");
    assert_eq!(v13, v9, "{label}: lockfile");

    let v9 = PnpmLockfile::from_bytes(v9_bytes).unwrap();
    let v13 = PnpmLockfile::from_bytes(v13_bytes).unwrap();
    for (path, importer) in &v9.importers {
        for name in importer.dependencies.all_dependency_names() {
            let (specifier, _) = importer.dependencies.find_resolution(name).unwrap();
            assert_eq!(
                v13.resolve_package(path, name, specifier).unwrap(),
                v9.resolve_package(path, name, specifier).unwrap(),
                "{label}: resolve_package({path}, {name}, {specifier})"
            );
        }
    }
    for key in v9.snapshots.iter().flat_map(|snapshots| snapshots.keys()) {
        assert_eq!(
            v13.all_dependencies(key).unwrap(),
            v9.all_dependencies(key).unwrap(),
            "{label}: all_dependencies({key})"
        );
    }
    assert_eq!(
        crate::all_transitive_closures(&v13, v13.test_workspaces(), true).unwrap(),
        crate::all_transitive_closures(&v9, v9.test_workspaces(), true).unwrap(),
        "{label}: closures"
    );
    assert_eq!(
        v13.global_fields().settings,
        v9.global_fields().settings,
        "{label}: global settings"
    );
}

/// Pairs whose v9 and v13 files pnpm wrote with different catalog versions,
/// so the catalogs of the two cannot agree.
const KNOWN_CATALOG_DIVERGENCE: &[(&str, &str)] = &[(
    "/catalogs",
    "pnpm 12.20.0 records the npm: alias entry lodash-compat as 4.17.21 in v13 and 4.18.1 in v9",
)];

fn corpus_pairs(root: &Path) -> Vec<(String, Vec<u8>, Vec<u8>)> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut pairs: Vec<_> = entries
        .filter_map(|entry| {
            let dir = entry.ok()?.path();
            let v9 = std::fs::read(dir.join("v9/pnpm-lock.yaml")).ok()?;
            let v13 = std::fs::read(dir.join("v13/pnpm-lock.yaml")).ok()?;
            Some((dir.display().to_string(), v9, v13))
        })
        .collect();
    pairs.sort();
    pairs
}

/// The full generated corpus, outside the repository. Run with
/// `V13_CORPUS=/tmp/v13-corpus/out cargo test -p turborepo-lockfiles --
/// --ignored`.
#[test]
#[ignore]
fn paired_oracle_on_external_corpus() {
    let root = std::env::var("V13_CORPUS").expect("V13_CORPUS names a corpus directory");
    let mut failures = Vec::new();
    for (label, v9, v13) in corpus_pairs(Path::new(&root)) {
        let result = std::panic::catch_unwind(|| assert_paired(&v9, &v13, &label));
        if result.is_err() {
            failures.push(label);
        }
    }
    assert!(
        failures.is_empty(),
        "paired oracle failed for {failures:#?}"
    );
}

/// `lockfile-tests/fixtures/pnpm-v13-*`, each with its v9 twin.
pub(super) fn fixture_pairs() -> Vec<(String, Vec<u8>, Vec<u8>)> {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lockfile-tests/fixtures");
    let mut pairs: Vec<_> = std::fs::read_dir(&fixtures)
        .unwrap()
        .filter_map(|entry| {
            let dir = entry.ok()?.path();
            let v9 = std::fs::read(dir.join("pnpm-lock.v9.yaml")).ok()?;
            let v13 = std::fs::read(dir.join("pnpm-lock.yaml")).ok()?;
            Some((dir.display().to_string(), v9, v13))
        })
        .collect();
    pairs.sort();
    assert!(pairs.len() >= 8, "expected the pnpm-v13 fixtures");
    pairs
}

#[test]
fn paired_oracle_on_fixtures() {
    for (label, v9, v13) in fixture_pairs() {
        assert_paired(&v9, &v13, &label);
    }
}

#[test]
fn detects_v13_documents() {
    for (label, v9, v13) in fixture_pairs() {
        assert!(super::declares_next_format(&v13), "{label}");
        assert!(!super::declares_next_format(&v9), "{label}");
    }
}
