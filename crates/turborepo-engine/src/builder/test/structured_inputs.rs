use turborepo_types::TaskInputs;

use super::*;

fn engine_with_root_and_app_config(
    root_config: serde_json::Value,
    app_config: serde_json::Value,
) -> Result<Engine<Built, TaskDefinition>, BuilderError> {
    let repo_root_dir = TempDir::new().unwrap();
    let repo_root = AbsoluteSystemPathBuf::try_from(repo_root_dir.path()).unwrap();
    let package_graph = mock_package_graph(&repo_root, package_jsons! { repo_root, "app" => [] });
    let loader = TestTurboJsonLoader::new(HashMap::from([
        (PackageName::Root, turbo_json(root_config)),
        (PackageName::from("app"), turbo_json(app_config)),
    ]));

    EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks(Some(Spanned::new(TaskName::from("build"))))
        .with_workspaces(vec![PackageName::from("app")])
        .build()
}

#[test]
fn package_extends_inputs_before_structured_normalization() {
    let engine = engine_with_root_and_app_config(
        json!({
            "tasks": {
                "build": {
                    "inputs": ["$TURBO_DEFAULT$", "!.output/**"],
                    "outputs": [".output/**"]
                }
            }
        }),
        json!({
            "extends": ["//"],
            "tasks": {
                "build": {
                    "inputs": [
                        "$TURBO_EXTENDS$",
                        { "mode": "jit", "globs": ["src/generated/**"] }
                    ]
                }
            }
        }),
    )
    .unwrap();

    let inputs = &task_definition(&engine, "app#build").inputs;
    assert!(inputs.default, "root default inputs must be inherited");
    assert_eq!(inputs.globs, ["!.output/**"]);
    assert_eq!(inputs.jit_globs, ["src/generated/**"]);
    assert!(!inputs.jit_default);
    assert!(inputs.eager);
}

#[test]
fn package_extends_rejects_duplicate_startup_after_normalization() {
    let error = engine_with_root_and_app_config(
        json!({
            "tasks": {
                "build": {
                    "inputs": ["$TURBO_DEFAULT$"],
                    "outputs": ["dist/**"]
                }
            }
        }),
        json!({
            "extends": ["//"],
            "tasks": {
                "build": {
                    "inputs": [
                        "$TURBO_EXTENDS$",
                        { "mode": "startup", "globs": ["src/**"] }
                    ]
                }
            }
        }),
    )
    .unwrap_err();

    let message = error.to_string();
    assert!(
        message.contains("Legacy input strings normalize to mode \"startup\""),
        "expected duplicate startup error, got:\n{message}"
    );
    assert!(
        message.contains("Use either legacy startup inputs"),
        "expected actionable guidance, got:\n{message}"
    );
}

fn app_build_inputs(root_build: serde_json::Value, app_inputs: serde_json::Value) -> TaskInputs {
    let engine = engine_with_root_and_app_config(
        json!({ "tasks": { "build": root_build } }),
        json!({
            "extends": ["//"],
            "tasks": { "build": { "inputs": app_inputs } }
        }),
    )
    .unwrap();
    task_definition(&engine, "app#build").inputs.clone()
}

#[test]
fn package_extends_keeps_default_files_when_root_has_no_inputs() {
    let inputs = app_build_inputs(json!({}), json!(["$TURBO_EXTENDS$", "../shared/**"]));
    assert!(
        inputs.default,
        "a root task without inputs hashes every package file; extending it must keep them"
    );
    assert_eq!(inputs.globs, ["../shared/**"]);
    assert!(inputs.eager);
}

#[test]
fn package_extends_keeps_default_files_when_root_inputs_are_empty() {
    let inputs = app_build_inputs(
        json!({ "inputs": [] }),
        json!(["$TURBO_EXTENDS$", "../shared/**"]),
    );
    assert!(
        inputs.default,
        "`\"inputs\": []` hashes every package file; extending it must keep them"
    );
    assert_eq!(inputs.globs, ["../shared/**"]);
    assert!(inputs.eager);
}

#[test]
fn package_extends_with_jit_input_keeps_startup_hashing_of_default_files() {
    let inputs = app_build_inputs(
        json!({}),
        json!([
            "$TURBO_EXTENDS$",
            { "mode": "jit", "globs": ["src/generated/**"] }
        ]),
    );
    assert!(inputs.default);
    assert!(inputs.globs.is_empty());
    assert_eq!(inputs.jit_globs, ["src/generated/**"]);
    assert!(
        inputs.eager,
        "package files must still be hashed at startup"
    );
}

#[test]
fn package_extends_explicit_root_globs_without_adding_defaults() {
    let inputs = app_build_inputs(
        json!({ "inputs": ["src/**"] }),
        json!(["$TURBO_EXTENDS$", "lib/**"]),
    );
    assert!(!inputs.default);
    assert_eq!(inputs.globs, ["src/**", "lib/**"]);
}

#[test]
fn package_inputs_without_extends_marker_replace_root_default() {
    let inputs = app_build_inputs(json!({}), json!(["lib/**"]));
    assert!(!inputs.default);
    assert_eq!(inputs.globs, ["lib/**"]);
}
