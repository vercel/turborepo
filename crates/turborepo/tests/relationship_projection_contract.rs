#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::{fs, path::Path};

use common::{combined_output, git, run_turbo, setup};
use serde_json::Value;

const FIXTURE: &str = "relationship_projection_contract";

fn setup_fixture(dir: &Path) {
    setup::setup_integration_test(dir, FIXTURE, "npm@10.5.0", false).unwrap();
}

fn turbo_json(dir: &Path, args: &[&str]) -> Value {
    let output = run_turbo(dir, args);
    assert!(
        output.status.success(),
        "turbo {args:?} failed:\n{}",
        combined_output(&output)
    );
    serde_json::from_slice(&output.stdout).expect("turbo emits JSON")
}

fn strings(value: &Value) -> Vec<String> {
    let mut values: Vec<_> = value
        .as_array()
        .expect("value is an array")
        .iter()
        .map(|value| value.as_str().expect("array item is a string").to_string())
        .collect();
    values.sort();
    values
}

fn task<'a>(dry_run: &'a Value, task_id: &str) -> &'a Value {
    dry_run["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|task| task["taskId"] == task_id)
        .unwrap_or_else(|| panic!("missing task {task_id}"))
}

fn task_hash<'a>(dry_run: &'a Value, task_id: &str) -> &'a str {
    task(dry_run, task_id)["hash"]
        .as_str()
        .expect("task hash is a string")
}

fn package_names(dry_run: &Value) -> Vec<String> {
    strings(&dry_run["packages"])
}

#[test]
fn dry_run_projects_only_graph_forming_relationships_into_task_graph_and_hashes() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_fixture(tempdir.path());

    let dry_run = turbo_json(
        tempdir.path(),
        &["run", "build", "--filter=app", "--dry=json"],
    );

    let mut task_ids: Vec<_> = dry_run["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| task["taskId"].as_str().unwrap().to_string())
        .collect();
    task_ids.sort();
    assert_eq!(
        task_ids,
        [
            "app#build",
            "dev-lib#build",
            "optional-lib#build",
            "prod-lib#build",
            "transitive-lib#build",
        ]
    );

    assert_eq!(
        strings(&task(&dry_run, "app#build")["dependencies"]),
        ["dev-lib#build", "optional-lib#build", "prod-lib#build"]
    );
    assert_eq!(
        strings(&task(&dry_run, "prod-lib#build")["dependencies"]),
        ["transitive-lib#build"]
    );
    assert_eq!(
        strings(&task(&dry_run, "transitive-lib#build")["dependents"]),
        ["prod-lib#build"]
    );
    assert_eq!(
        task(&dry_run, "app#build")["resolvedTaskDefinition"]["dependsOn"],
        serde_json::json!(["^build"])
    );

    let app_inputs = task(&dry_run, "app#build")["inputs"].as_object().unwrap();
    let mut app_input_paths = app_inputs.keys().map(String::as_str).collect::<Vec<_>>();
    app_input_paths.sort();
    assert_eq!(app_input_paths, ["index.js", "package.json"]);

    let initial_app_hash = task_hash(&dry_run, "app#build").to_string();
    fs::write(
        tempdir.path().join("packages/required-peer/index.js"),
        "export const requiredPeer = \"changed peer\";\n",
    )
    .unwrap();
    let peer_change = turbo_json(
        tempdir.path(),
        &["run", "build", "--filter=app", "--dry=json"],
    );
    assert_eq!(
        task_hash(&peer_change, "app#build"),
        initial_app_hash,
        "peer-only package inputs must not affect the app task hash"
    );

    fs::write(
        tempdir.path().join("packages/transitive-lib/index.js"),
        "export const transitive = \"changed dependency\";\n",
    )
    .unwrap();
    let dependency_change = turbo_json(
        tempdir.path(),
        &["run", "build", "--filter=app", "--dry=json"],
    );
    assert!(
        task_hash(&dependency_change, "app#build") != initial_app_hash,
        "transitive graph dependency inputs must affect the app task hash"
    );
}

#[test]
fn filters_expand_dependency_and_reverse_dependent_relationships() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_fixture(tempdir.path());

    let dependencies = turbo_json(
        tempdir.path(),
        &["run", "build", "--filter=app...", "--dry=json"],
    );
    let dependents = turbo_json(
        tempdir.path(),
        &["run", "build", "--filter=...transitive-lib", "--dry=json"],
    );

    assert_eq!(
        package_names(&dependencies),
        [
            "app",
            "dev-lib",
            "optional-lib",
            "prod-lib",
            "root-lib",
            "transitive-lib"
        ]
    );
    assert_eq!(
        package_names(&dependents),
        ["app", "prod-lib", "transitive-lib"]
    );
}

#[test]
fn affectedness_propagates_through_reverse_relationships() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_fixture(tempdir.path());
    git(tempdir.path(), &["checkout", "-b", "relationship-change"]);
    fs::write(
        tempdir.path().join("packages/transitive-lib/index.js"),
        "export const transitive = \"changed transitive dependency\";\n",
    )
    .unwrap();
    git(tempdir.path(), &["add", "."]);
    git(
        tempdir.path(),
        &["commit", "-m", "change transitive-lib", "--quiet"],
    );

    let affected = turbo_json(
        tempdir.path(),
        &["run", "build", "--affected", "--dry=json"],
    );

    assert_eq!(
        package_names(&affected),
        ["app", "prod-lib", "transitive-lib"]
    );
}

#[test]
fn prune_preserves_install_relationships_and_applies_production_mode() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_fixture(tempdir.path());

    let output = run_turbo(tempdir.path(), &["prune", "app", "--out-dir=out-all"]);
    assert!(
        output.status.success(),
        "normal prune failed:\n{}",
        combined_output(&output)
    );
    let output = run_turbo(
        tempdir.path(),
        &["prune", "app", "--production", "--out-dir=out-production"],
    );
    assert!(
        output.status.success(),
        "production prune failed:\n{}",
        combined_output(&output)
    );

    assert_eq!(
        pruned_packages(&tempdir.path().join("out-all")),
        [
            "app",
            "dev-lib",
            "optional-lib",
            "prod-lib",
            "required-peer",
            "root-lib",
            "transitive-lib",
        ]
    );
    assert_eq!(
        pruned_packages(&tempdir.path().join("out-production")),
        [
            "app",
            "optional-lib",
            "prod-lib",
            "required-peer",
            "root-lib",
            "transitive-lib",
        ]
    );

    for out_dir in ["out-all", "out-production"] {
        let lockfile: Value = serde_json::from_slice(
            &fs::read(tempdir.path().join(out_dir).join("package-lock.json")).unwrap(),
        )
        .unwrap();
        let mut workspace_keys = lockfile["packages"]
            .as_object()
            .unwrap()
            .keys()
            .filter(|key| {
                key.is_empty() || key.starts_with("apps/") || key.starts_with("packages/")
            })
            .map(String::as_str)
            .collect::<Vec<_>>();
        workspace_keys.sort();
        let expected = if out_dir == "out-all" {
            vec![
                "",
                "apps/app",
                "packages/dev-lib",
                "packages/optional-lib",
                "packages/prod-lib",
                "packages/required-peer",
                "packages/root-lib",
                "packages/transitive-lib",
            ]
        } else {
            vec![
                "",
                "apps/app",
                "packages/optional-lib",
                "packages/prod-lib",
                "packages/required-peer",
                "packages/root-lib",
                "packages/transitive-lib",
            ]
        };
        assert_eq!(workspace_keys, expected);
        assert_eq!(
            lockfile["packages"]["packages/prod-lib"]["dependencies"]["transitive-lib"], "*",
            "{out_dir} must retain the relationship that brings transitive-lib into the closure"
        );
    }
}

fn pruned_packages(out_dir: &Path) -> Vec<String> {
    let mut packages = Vec::new();
    for parent in ["apps", "packages"] {
        let parent = out_dir.join(parent);
        if !parent.exists() {
            continue;
        }
        packages.extend(
            fs::read_dir(parent)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()),
        );
    }
    packages.sort();
    packages
}

#[test]
fn query_exposes_exact_package_edges_including_the_root_workspace_dependency() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_fixture(tempdir.path());

    let query = turbo_json(
        tempdir.path(),
        &[
            "query",
            "query { packages { items { name directDependencies { items { name } } \
             directDependents { items { name } } } } }",
        ],
    );
    let mut edges = Vec::new();
    let mut reverse_edges = Vec::new();
    for package in query["data"]["packages"]["items"].as_array().unwrap() {
        let name = package["name"].as_str().unwrap();
        for dependency in package["directDependencies"]["items"].as_array().unwrap() {
            edges.push(format!(
                "{name} -> {}",
                dependency["name"].as_str().unwrap()
            ));
        }
        for dependent in package["directDependents"]["items"].as_array().unwrap() {
            reverse_edges.push(format!("{name} <- {}", dependent["name"].as_str().unwrap()));
        }
    }
    edges.sort();
    reverse_edges.sort();

    // The public dependency view includes the structural root fallback for
    // leaf packages. The dependent view reports only manifest relationships.
    assert_eq!(
        edges,
        [
            "// -> //",
            "// -> root-lib",
            "app -> dev-lib",
            "app -> optional-lib",
            "app -> prod-lib",
            "dev-lib -> //",
            "optional-lib -> //",
            "optional-peer -> //",
            "prod-lib -> transitive-lib",
            "required-peer -> //",
            "root-lib -> //",
            "transitive-lib -> //",
        ]
    );
    assert_eq!(
        reverse_edges,
        [
            "dev-lib <- app",
            "optional-lib <- app",
            "prod-lib <- app",
            "root-lib <- //",
            "transitive-lib <- prod-lib",
        ]
    );

    let app = turbo_json(
        tempdir.path(),
        &[
            "query",
            "query { package(name: \"app\") { directDependencies { items { name } } \
             directDependents { items { name } } } }",
        ],
    );
    assert_eq!(
        relation_names(&app["data"]["package"], "directDependencies"),
        ["dev-lib", "optional-lib", "prod-lib"]
    );
    assert!(relation_names(&app["data"]["package"], "directDependents").is_empty());
}

fn relation_names(package: &Value, relationship: &str) -> Vec<String> {
    let names: Vec<Value> = package[relationship]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["name"].clone())
        .collect();
    strings(&Value::Array(names))
}
