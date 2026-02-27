mod common;

use std::fs;

use common::{run_turbo, setup};

fn set_engines(dir: &std::path::Path, node_version: &str) {
    let pkg_path = dir.join("package.json");
    let contents = fs::read_to_string(&pkg_path).unwrap();
    let mut pkg: serde_json::Value = serde_json::from_str(&contents).unwrap();
    pkg["engines"] = serde_json::json!({ "node": node_version });
    fs::write(
        &pkg_path,
        serde_json::to_string_pretty(&pkg).unwrap() + "\n",
    )
    .unwrap();
}

#[test]
fn test_engines_affect_hash() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    // Set engines to >=12
    set_engines(tempdir.path(), ">=12");

    let output1 = run_turbo(tempdir.path(), &["build", "--dry=json", "--filter=my-app"]);
    let json1: serde_json::Value = serde_json::from_slice(&output1.stdout).unwrap();
    let hash1 = json1["tasks"].as_array().unwrap().last().unwrap()["hash"]
        .as_str()
        .unwrap()
        .to_string();

    // Change engines to >=16
    set_engines(tempdir.path(), ">=16");

    let output2 = run_turbo(tempdir.path(), &["build", "--dry=json", "--filter=my-app"]);
    let json2: serde_json::Value = serde_json::from_slice(&output2.stdout).unwrap();
    let hash2 = json2["tasks"].as_array().unwrap().last().unwrap()["hash"]
        .as_str()
        .unwrap()
        .to_string();

    assert_ne!(hash1, hash2, "hash should change when engines change");
}

#[test]
fn test_engines_in_global_cache_inputs() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    set_engines(tempdir.path(), ">=16");

    let output = run_turbo(tempdir.path(), &["build", "--dry=json"]);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    let engines = &json["globalCacheInputs"]["engines"];
    assert_eq!(
        engines,
        &serde_json::json!({ "node": ">=16" }),
        "engines should appear in global cache inputs"
    );
}
