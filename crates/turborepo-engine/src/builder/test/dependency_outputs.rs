use super::*;

fn dependency_outputs_engine(
    package_jsons_for_repo: impl FnOnce(
        &AbsoluteSystemPathBuf,
    ) -> HashMap<AbsoluteSystemPathBuf, PackageJson>,
    tasks: serde_json::Value,
    workspaces: &[&str],
) -> Result<Engine<Built, TaskDefinition>, BuilderError> {
    let repo_root_dir = TempDir::new().unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(&repo_root, package_jsons_for_repo(&repo_root));
    let loader = TestTurboJsonLoader::new(HashMap::from([(
        PackageName::Root,
        turbo_json(json!({ "tasks": tasks })),
    )]));

    EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_workspaces(
            workspaces
                .iter()
                .map(|workspace| PackageName::from(*workspace))
                .collect(),
        )
        .with_tasks(Some(Spanned::new(TaskName::from("build"))))
        .build()
}

fn sorted_task_names(tasks: Vec<TaskId<'static>>) -> Vec<String> {
    let mut names = tasks
        .into_iter()
        .map(|task| task.to_string())
        .collect::<Vec<_>>();
    names.sort();
    names
}

#[test]
fn dependency_outputs_without_from_selects_direct_task_dependencies() {
    let engine = dependency_outputs_engine(
        |repo_root| {
            package_jsons! {
                repo_root,
                "app" => ["util"],
                "util" => ["dep"],
                "dep" => []
            }
        },
        json!({
            "codegen": { "outputs": ["src/generated/**"] },
            "build": {
                "dependsOn": ["codegen", "^build"],
                "inputs": [{ "mode": "dependencyOutputs" }],
                "outputs": ["dist/**"]
            }
        }),
        &["app", "util", "dep"],
    )
    .unwrap();

    let selected = engine.dependency_output_producers(&TaskId::new("app", "build"), None);
    assert_eq!(sorted_task_names(selected), ["app#codegen", "util#build"]);
}

#[test]
fn dependency_outputs_from_topological_selector_selects_transitive_tasks() {
    let engine = dependency_outputs_engine(
        |repo_root| {
            package_jsons! {
                repo_root,
                "app" => ["util"],
                "util" => ["dep"],
                "dep" => []
            }
        },
        json!({
            "build": {
                "dependsOn": ["^build"],
                "inputs": [{ "mode": "dependencyOutputs", "from": ["^build"] }],
                "outputs": ["dist/**"]
            }
        }),
        &["app", "util", "dep"],
    )
    .unwrap();

    let selected = engine
        .dependency_output_producers(&TaskId::new("app", "build"), Some(&["^build".to_string()]));
    assert_eq!(sorted_task_names(selected), ["dep#build", "util#build"]);
}

#[test]
fn dependency_outputs_allows_empty_topological_selector_when_no_tasks_match() {
    let engine = dependency_outputs_engine(
        |repo_root| package_jsons! { repo_root, "app" => [] },
        json!({
            "build": { "dependsOn": ["^build"], "outputs": ["dist/**"] },
            "app#build": {
                "dependsOn": ["^build"],
                "inputs": [{ "mode": "dependencyOutputs", "from": ["^build"] }]
            }
        }),
        &["app"],
    )
    .unwrap();

    assert!(
        engine
            .dependency_output_producers_for_selector(&TaskId::new("app", "build"), "^build")
            .is_empty()
    );
}

#[test]
fn dependency_outputs_rejects_selector_without_matching_dependency_task() {
    let error = dependency_outputs_engine(
        |repo_root| {
            package_jsons! {
                repo_root,
                "app" => ["util"],
                "util" => []
            }
        },
        json!({
            "build": { "dependsOn": ["^build"], "outputs": ["dist/**"] },
            "app#build": {
                "dependsOn": ["^build"],
                "inputs": [{ "mode": "dependencyOutputs", "from": ["^codegen"] }]
            }
        }),
        &["app", "util"],
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("does not match any eligible dependency task node"),
        "expected selector validation error, got {error}"
    );
}

#[test]
fn dependency_outputs_from_package_qualified_selector_selects_matching_task() {
    let engine = dependency_outputs_engine(
        |repo_root| {
            package_jsons! {
                repo_root,
                "app" => ["util"],
                "util" => ["dep"],
                "dep" => []
            }
        },
        json!({
            "build": { "dependsOn": ["^build"], "outputs": ["dist/**"] },
            "app#build": {
                "dependsOn": ["^build"],
                "inputs": [{ "mode": "dependencyOutputs", "from": ["util#build"] }]
            }
        }),
        &["app", "util", "dep"],
    )
    .unwrap();

    let selected =
        engine.dependency_output_producers_for_selector(&TaskId::new("app", "build"), "util#build");
    assert_eq!(sorted_task_names(selected), ["util#build"]);
}

#[test]
fn dependency_outputs_from_can_select_a_transitive_task_in_the_same_package() {
    let engine = dependency_outputs_engine(
        |repo_root| package_jsons! { repo_root, "app" => [] },
        json!({
            "codegen": { "outputs": ["src/generated/**"] },
            "generate": {
                "dependsOn": ["codegen"],
                "outputs": ["src/generated-wrapper/**"]
            },
            "build": {
                "dependsOn": ["generate"],
                "inputs": [{ "mode": "dependencyOutputs", "from": ["codegen"] }],
                "outputs": ["dist/**"]
            }
        }),
        &["app"],
    )
    .unwrap();

    let selected =
        engine.dependency_output_producers_for_selector(&TaskId::new("app", "build"), "codegen");
    assert_eq!(sorted_task_names(selected), ["app#codegen"]);
}

#[test]
fn dependency_outputs_rejects_globs_not_covered_by_selected_outputs() {
    let error = dependency_outputs_engine(
        |repo_root| package_jsons! { repo_root, "app" => [] },
        json!({
            "codegen": { "outputs": ["src/generated/**"] },
            "build": { "dependsOn": ["codegen"], "outputs": ["dist/**"] },
            "app#build": {
                "dependsOn": ["codegen"],
                "inputs": [{
                    "mode": "dependencyOutputs",
                    "from": ["codegen"],
                    "globs": ["src/generated/**", "private.txt"]
                }]
            }
        }),
        &["app"],
    )
    .unwrap_err();

    let message = error.to_string();
    assert!(message.contains("dependencyOutputs.globs"), "got {message}");
    assert!(message.contains("private.txt"), "got {message}");
}

#[test]
fn dependency_outputs_rejects_selected_tasks_without_declared_outputs() {
    let error = dependency_outputs_engine(
        |repo_root| package_jsons! { repo_root, "app" => [] },
        json!({
            "codegen": {},
            "build": { "dependsOn": ["codegen"], "outputs": ["dist/**"] },
            "app#build": {
                "dependsOn": ["codegen"],
                "inputs": [{ "mode": "dependencyOutputs", "from": ["codegen"] }]
            }
        }),
        &["app"],
    )
    .unwrap_err();

    let message = error.to_string();
    assert!(
        message.contains("does not declare outputs"),
        "got {message}"
    );
    assert!(message.contains("Add outputs to"), "got {message}");
    assert!(
        message.contains("or remove it from dependencyOutputs.from"),
        "got {message}"
    );
}
