use std::path::Path;

use pretty_assertions::assert_eq;

use super::super::{Format, PnpmLockfile};
use crate::Lockfile;

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

fn assert_round_trip(v13: &[u8], label: &str) {
    let lockfile = PnpmLockfile::from_bytes(v13).unwrap();
    let encoded = lockfile.encode().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&encoded),
        String::from_utf8_lossy(v13),
        "{label}: encode(read(x)) != x"
    );
}

#[test]
fn round_trips_fixtures() {
    for (label, _, v13) in fixture_pairs() {
        assert_round_trip(&v13, &label);
    }
}

#[test]
#[ignore]
fn round_trips_external_corpus() {
    let root = std::env::var("V13_CORPUS").expect("V13_CORPUS names a corpus directory");
    let mut failures = Vec::new();
    for (label, _, v13) in corpus_pairs(Path::new(&root)) {
        if std::panic::catch_unwind(|| assert_round_trip(&v13, &label)).is_err() {
            failures.push(label);
        }
    }
    assert!(failures.is_empty(), "round trip failed for {failures:#?}");
}

#[test]
fn detects_v13_documents() {
    for (label, v9, v13) in fixture_pairs() {
        assert!(super::declares_next_format(&v13), "{label}");
        assert!(!super::declares_next_format(&v9), "{label}");
    }
}

fn prune_inputs(lockfile: &PnpmLockfile, target: &str) -> (Vec<String>, Vec<String>) {
    let mut workspaces = std::collections::BTreeSet::from([target.to_string()]);
    let mut queue = vec![target.to_string()];
    while let Some(id) = queue.pop() {
        let importer = &lockfile.importers[&id];
        for name in importer.dependencies.all_dependency_names() {
            let (_, version) = importer.dependencies.find_resolution(name).unwrap();
            let dependency = if let Some(path) = version.strip_prefix("link:") {
                normalize(&format!("{id}/{path}"))
            } else if let Some(path) = version.strip_prefix("file:") {
                normalize(super::key::without_peer(path))
            } else {
                continue;
            };
            if lockfile.importers.contains_key(&dependency) && workspaces.insert(dependency.clone())
            {
                queue.push(dependency);
            }
        }
    }
    workspaces.remove(".");
    let all = lockfile.test_workspaces();
    let selected = all
        .into_iter()
        .filter(|(id, _)| id == "." || workspaces.contains(id))
        .collect();
    let closures = crate::all_transitive_closures(lockfile, selected, true).unwrap();
    let mut packages: Vec<String> = closures
        .values()
        .flatten()
        .map(|package| package.key.clone())
        .collect();
    packages.sort();
    packages.dedup();
    (workspaces.into_iter().collect(), packages)
}

fn normalize(path: &str) -> String {
    let mut segments: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            segment => segments.push(segment),
        }
    }
    if segments.is_empty() {
        ".".to_string()
    } else {
        segments.join("/")
    }
}

fn pruned(lockfile: &PnpmLockfile, workspaces: &[String], packages: &[String]) -> PnpmLockfile {
    let pruned = lockfile.subgraph(workspaces, packages).unwrap();
    (pruned.as_ref() as &dyn std::any::Any)
        .downcast_ref::<PnpmLockfile>()
        .unwrap()
        .clone()
}

pub(super) fn assert_prune_parity(v9_bytes: &[u8], v13_bytes: &[u8], label: &str) {
    let v9 = PnpmLockfile::from_bytes(v9_bytes).unwrap();
    let v13 = PnpmLockfile::from_bytes(v13_bytes).unwrap();
    let mut targets: Vec<&String> = v9.importers.keys().filter(|id| *id != ".").collect();
    targets.sort();
    for target in targets {
        let label = &format!("{label} -> {target}");
        let (workspaces, packages) = prune_inputs(&v9, target);
        let pruned_v9 = pruned(&v9, &workspaces, &packages);
        let pruned_v13 = pruned(&v13, &workspaces, &packages);
        let bytes = crate::Lockfile::encode(&pruned_v13).unwrap();

        let reread = PnpmLockfile::from_bytes(&bytes).unwrap();
        assert_eq!(
            reread, pruned_v13,
            "{label}: pruned bytes read back as the pruned model"
        );
        let (expected, actual) = normalized_models(pruned_v9, pruned_v13);
        assert_eq!(actual.importers, expected.importers, "{label}: importers");
        assert_eq!(actual.snapshots, expected.snapshots, "{label}: snapshots");
        assert_eq!(actual.packages, expected.packages, "{label}: packages");
        assert_eq!(actual, expected, "{label}: lockfile");

        let again = crate::Lockfile::encode(&pruned(&reread, &workspaces, &packages)).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&again),
            String::from_utf8_lossy(&bytes),
            "{label}: prune is idempotent"
        );
    }
}

fn normalized_models(mut v9: PnpmLockfile, mut v13: PnpmLockfile) -> (PnpmLockfile, PnpmLockfile) {
    for package in v9
        .packages
        .iter_mut()
        .flat_map(|packages| packages.values_mut())
    {
        package.other.remove("deprecated");
    }
    for package in [&mut v9, &mut v13]
        .into_iter()
        .flat_map(|lockfile| lockfile.packages.iter_mut().flat_map(|p| p.values_mut()))
    {
        if package.resolution.other.contains_key("gitHosted") {
            package.resolution.integrity = None;
        }
    }
    if v9
        .snapshots
        .as_ref()
        .is_some_and(|snapshots| snapshots.is_empty())
    {
        v9.snapshots = None;
    }
    v13.lockfile_version = v9.lockfile_version.clone();
    v13.format = Format::Suffixed;
    v9.dependency_index = Default::default();
    v13.dependency_index = Default::default();
    (v9, v13)
}

#[test]
fn prune_parity_on_fixtures() {
    for (label, v9, v13) in fixture_pairs() {
        assert_prune_parity(&v9, &v13, &label);
    }
}

#[test]
#[ignore]
fn prune_parity_on_external_corpus() {
    let root = std::env::var("V13_CORPUS").expect("V13_CORPUS names a corpus directory");
    let mut failures = Vec::new();
    for (label, v9, v13) in corpus_pairs(Path::new(&root)) {
        if KNOWN_CATALOG_DIVERGENCE
            .iter()
            .any(|(name, _)| label.ends_with(name))
        {
            continue;
        }
        if std::panic::catch_unwind(|| assert_prune_parity(&v9, &v13, &label)).is_err() {
            failures.push(label);
        }
    }
    assert!(failures.is_empty(), "prune parity failed for {failures:#?}");
}

#[test]
fn prune_to_every_workspace_is_identity() {
    for (label, _, v13) in fixture_pairs() {
        let lockfile = PnpmLockfile::from_bytes(&v13).unwrap();
        let workspaces: Vec<String> = lockfile
            .importers
            .keys()
            .filter(|id| *id != ".")
            .cloned()
            .collect();
        let closures =
            crate::all_transitive_closures(&lockfile, lockfile.test_workspaces(), true).unwrap();
        let mut packages: Vec<String> = closures
            .values()
            .flatten()
            .map(|package| package.key.clone())
            .collect();
        packages.sort();
        packages.dedup();
        let encoded = crate::Lockfile::encode(&pruned(&lockfile, &workspaces, &packages)).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&encoded),
            String::from_utf8_lossy(&v13),
            "{label}: pruning to every workspace changed the lockfile"
        );
    }
}

fn linked_above_drift_document(p: &str, b_p: &str) -> String {
    format!(
        r#"{{
	"lockfileVersion": "13.0",
	"projects": {{
		"a": {{"dependencies": {{"p": "1.0.0", "x": "1.0.0", "y": "1.0.0", "z": "1.0.0"}}}},
		"b": {{"dependencies": {{"p": {b_p}, "x": "1.0.0", "y": "2.0.0", "z": "2.0.0"}}}},
	}},
	"packages": {{
		"p@1.0.0": {p},
		"x@1.0.0": {{
			"integrity": "sha512-x",
			"peerDependencies": {{"y": "*"}},
			"snapshots": [
				{{"peerDependencies": {{"y": {{"version": "1.0.0"}}}}}},
				{{"peerDependencies": {{"y": {{"version": "2.0.0"}}}}}},
			],
		}},
		"y@1.0.0": {{"integrity": "sha512-y1"}},
		"y@2.0.0": {{"integrity": "sha512-y2"}},
		"z@1.0.0": {{"integrity": "sha512-z1"}},
		"z@2.0.0": {{"integrity": "sha512-z2"}},
	}},
}}
"#
    )
}

fn assert_pins_linked_above_drift(document: &str, expected_key: &str) {
    let lockfile = PnpmLockfile::from_bytes(document.as_bytes()).unwrap();
    assert!(
        lockfile
            .snapshots
            .as_ref()
            .unwrap()
            .contains_key(expected_key),
        "{expected_key} in {:?}",
        lockfile.snapshots.as_ref().unwrap().keys()
    );
    let workspaces = vec!["b".to_string()];
    let closures = crate::all_transitive_closures(
        &lockfile,
        lockfile
            .test_workspaces()
            .into_iter()
            .filter(|(id, _)| id == "b")
            .collect(),
        true,
    )
    .unwrap();
    let packages: Vec<String> = closures
        .values()
        .flatten()
        .map(|package| package.key.clone())
        .collect();
    let expected = lockfile.subgraph_suffixed(&workspaces, &packages).unwrap();
    let pruned = pruned(&lockfile, &workspaces, &packages);
    let bytes = crate::Lockfile::encode(&pruned).unwrap();
    let text = String::from_utf8(bytes.clone()).unwrap();
    let pinned = PnpmLockfile::from_bytes(&bytes).unwrap();
    let Format::Next(next) = &pinned.format else {
        panic!("a pruned v13 lockfile stays v13");
    };
    let package = &next.tree["packages"]["p@1.0.0"];
    let binding = match package.get("snapshots") {
        Some(snapshots) => &snapshots[0]["peerDependencies"]["x"],
        None => &package["peerDependencies"]["x"],
    };
    assert_eq!(
        binding["with"]["y"], "1.0.0",
        "the binding is pinned:\n{text}"
    );
    let reread = PnpmLockfile::from_bytes(&bytes).unwrap();
    assert_eq!(reread.importers, expected.importers);
    assert_eq!(reread.snapshots, expected.snapshots);
    assert!(reread.snapshots.unwrap().contains_key(expected_key));
}

#[test]
fn pins_a_binding_whose_provider_drifts() {
    let document = linked_above_drift_document(
        r#"{"integrity": "sha512-p", "peerDependencies": {"x": {"specifier": "^1.0.0", "version": "1.0.0"}}}"#,
        r#""1.0.0""#,
    );
    assert_pins_linked_above_drift(&document, "p@1.0.0(x@1.0.0(y@1.0.0))");
}

#[test]
fn dehoists_a_shared_binding_to_pin_it() {
    let document = linked_above_drift_document(
        r#"{
			"integrity": "sha512-p",
			"peerDependencies": {"x": {"specifier": "^1.0.0", "version": "1.0.0"}, "z": "*"},
			"snapshots": [
				{"peerDependencies": {"z": {"version": "1.0.0"}}},
				{"peerDependencies": {"z": {"version": "2.0.0"}}},
			],
		}"#,
        r#"{"specifier": "1.0.0", "with": {"z": "1.0.0"}}"#,
    );
    assert_pins_linked_above_drift(&document, "p@1.0.0(x@1.0.0(y@1.0.0))(z@1.0.0)");
}

#[test]
fn merged_lockfiles_refuse_to_encode() {
    let (_, _, v13) = fixture_pairs().remove(0);
    let mut lockfile = PnpmLockfile::from_bytes(&v13).unwrap();
    lockfile.merge_per_workspace_lockfiles(&[]).unwrap();
    assert!(matches!(lockfile.format, Format::MergedNext));
    assert!(lockfile.encode().is_err());
    let pruned = lockfile.subgraph(&[], &[]).unwrap();
    assert!(pruned.encode().is_err());
}
