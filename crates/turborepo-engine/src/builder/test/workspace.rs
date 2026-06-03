use super::*;

#[test]
fn test_workspace_config_overrides_task_fields_and_deps() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "override-values" => []
        },
    );
    let turbo_jsons = vec![
        (
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "override-values-task": {
                        "inputs": ["src/foo.txt"],
                        "outputs": ["out/**"],
                        "env": ["SOME_VAR"],
                        "outputLogs": "new-only"
                    },
                    "override-values-task-with-deps": {
                        "dependsOn": [
                            "override-values-underlying-task",
                            "^override-values-underlying-topo-task"
                        ]
                    },
                    "override-values-underlying-task": {},
                    "override-values-underlying-topo-task": {}
                }
            })),
        ),
        (
            PackageName::from("override-values"),
            turbo_json(json!({
                "extends": ["//"],
                "tasks": {
                    "override-values-task": {
                        "inputs": ["src/bar.txt"],
                        "outputs": ["lib/**"],
                        "env": ["OTHER_VAR"],
                        "outputLogs": "full"
                    },
                    "override-values-task-with-deps": {
                        "dependsOn": []
                    }
                }
            })),
        ),
    ]
    .into_iter()
    .collect();
    let loader = TestTurboJsonLoader::new(turbo_jsons);

    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks(vec![
            Spanned::new(TaskName::from("override-values-task")),
            Spanned::new(TaskName::from("override-values-task-with-deps")),
        ])
        .with_workspaces(vec![PackageName::from("override-values")])
        .build()
        .unwrap();

    let override_def = task_definition(&engine, "override-values#override-values-task");
    assert_eq!(override_def.inputs.globs, vec!["src/bar.txt"]);
    assert_eq!(override_def.outputs.inclusions, vec!["lib/**"]);
    assert_eq!(override_def.env, vec!["OTHER_VAR"]);
    assert_eq!(override_def.output_logs, OutputLogsMode::Full);

    let deps_def = task_definition(&engine, "override-values#override-values-task-with-deps");
    assert!(deps_def.task_dependencies.is_empty());
    assert!(deps_def.topological_dependencies.is_empty());
    assert_eq!(
        all_dependencies(&engine),
        deps! {
            "override-values#override-values-task" => ["___ROOT___"],
            "override-values#override-values-task-with-deps" => ["___ROOT___"]
        }
    );
}

#[test]
fn test_workspace_config_adds_task_fields_and_new_tasks() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "add-keys" => [],
            "add-tasks" => []
        },
    );
    let turbo_jsons = vec![
        (
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "add-keys-task": {},
                    "add-keys-underlying-task": {}
                }
            })),
        ),
        (
            PackageName::from("add-keys"),
            turbo_json(json!({
                "extends": ["//"],
                "tasks": {
                    "add-keys-task": {
                        "dependsOn": ["add-keys-underlying-task"],
                        "inputs": ["src/foo.txt"],
                        "outputs": ["out/**"],
                        "env": ["SOME_VAR"],
                        "outputLogs": "new-only"
                    },
                    "add-keys-underlying-task": {}
                }
            })),
        ),
        (
            PackageName::from("add-tasks"),
            turbo_json(json!({
                "extends": ["//"],
                "tasks": {
                    "added-task": {
                        "outputs": ["out/**"]
                    }
                }
            })),
        ),
    ]
    .into_iter()
    .collect();
    let loader = TestTurboJsonLoader::new(turbo_jsons);

    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks(vec![
            Spanned::new(TaskName::from("add-keys#add-keys-task")),
            Spanned::new(TaskName::from("add-tasks#added-task")),
        ])
        .with_workspaces(vec![
            PackageName::from("add-keys"),
            PackageName::from("add-tasks"),
        ])
        .build()
        .unwrap();

    let add_keys_def = task_definition(&engine, "add-keys#add-keys-task");
    assert_eq!(
        task_names(&add_keys_def.task_dependencies),
        vec!["add-keys-underlying-task"]
    );
    assert_eq!(add_keys_def.inputs.globs, vec!["src/foo.txt"]);
    assert_eq!(add_keys_def.outputs.inclusions, vec!["out/**"]);
    assert_eq!(add_keys_def.env, vec!["SOME_VAR"]);
    assert_eq!(add_keys_def.output_logs, OutputLogsMode::NewOnly);

    let added_def = task_definition(&engine, "add-tasks#added-task");
    assert_eq!(added_def.outputs.inclusions, vec!["out/**"]);
    assert_eq!(
        all_dependencies(&engine),
        deps! {
            "add-keys#add-keys-task" => ["add-keys#add-keys-underlying-task"],
            "add-keys#add-keys-underlying-task" => ["___ROOT___"],
            "add-tasks#added-task" => ["___ROOT___"]
        }
    );
}

#[test]
fn test_workspace_config_overrides_persistent_and_cache_flags() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "persistent" => [],
            "cached" => []
        },
    );
    let turbo_jsons = vec![
        (
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "persistent-task-1": { "persistent": true },
                    "persistent-task-2": { "persistent": true },
                    "persistent-task-3": { "persistent": true },
                    "persistent-task-4": {},
                    "cached-task-1": { "cache": false, "outputs": ["out/**"] },
                    "cached-task-2": { "cache": true, "outputs": ["out/**"] },
                    "cached-task-3": { "outputs": ["out/**"] }
                }
            })),
        ),
        (
            PackageName::from("persistent"),
            turbo_json(json!({
                "extends": ["//"],
                "tasks": {
                    "persistent-task-2": { "persistent": false },
                    "persistent-task-3": {},
                    "persistent-task-4": { "persistent": true }
                }
            })),
        ),
        (
            PackageName::from("cached"),
            turbo_json(json!({
                "extends": ["//"],
                "tasks": {
                    "cached-task-1": { "cache": true },
                    "cached-task-2": { "cache": false },
                    "cached-task-3": { "cache": false }
                }
            })),
        ),
    ]
    .into_iter()
    .collect();
    let loader = TestTurboJsonLoader::new(turbo_jsons);

    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks(vec![
            Spanned::new(TaskName::from("persistent-task-1")),
            Spanned::new(TaskName::from("persistent-task-2")),
            Spanned::new(TaskName::from("persistent-task-3")),
            Spanned::new(TaskName::from("persistent-task-4")),
            Spanned::new(TaskName::from("cached-task-1")),
            Spanned::new(TaskName::from("cached-task-2")),
            Spanned::new(TaskName::from("cached-task-3")),
        ])
        .with_workspaces(vec![
            PackageName::from("persistent"),
            PackageName::from("cached"),
        ])
        .build()
        .unwrap();

    assert!(task_definition(&engine, "persistent#persistent-task-1").persistent);
    assert!(!task_definition(&engine, "persistent#persistent-task-2").persistent);
    assert!(task_definition(&engine, "persistent#persistent-task-3").persistent);
    assert!(task_definition(&engine, "persistent#persistent-task-4").persistent);

    assert!(task_definition(&engine, "cached#cached-task-1").cache);
    assert!(!task_definition(&engine, "cached#cached-task-2").cache);
    assert!(!task_definition(&engine, "cached#cached-task-3").cache);
}

#[test]
fn test_workspace_config_dependency_resolution_and_cross_workspace_deps() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "blank-pkg" => [],
            "missing-workspace-config" => ["blank-pkg"],
            "omit-keys" => ["blank-pkg"],
            "override-values" => ["blank-pkg"],
            "cross-workspace" => ["blank-pkg"]
        },
    );
    let turbo_jsons = vec![
        (
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "missing-workspace-config-task-with-deps": {
                        "dependsOn": [
                            "missing-workspace-config-underlying-task",
                            "^missing-workspace-config-underlying-topo-task"
                        ]
                    },
                    "missing-workspace-config-underlying-task": {},
                    "missing-workspace-config-underlying-topo-task": {},
                    "omit-keys-task-with-deps": {
                        "dependsOn": [
                            "omit-keys-underlying-task",
                            "^omit-keys-underlying-topo-task"
                        ]
                    },
                    "omit-keys-underlying-task": {},
                    "omit-keys-underlying-topo-task": {},
                    "override-values-task-with-deps": {
                        "dependsOn": [
                            "override-values-underlying-task",
                            "^override-values-underlying-topo-task"
                        ]
                    },
                    "override-values-underlying-task": {},
                    "override-values-underlying-topo-task": {},
                    "cross-workspace-task": {},
                    "cross-workspace-underlying-task": {}
                }
            })),
        ),
        (
            PackageName::from("blank-pkg"),
            turbo_json(json!({ "extends": ["//"], "tasks": {} })),
        ),
        (
            PackageName::from("omit-keys"),
            turbo_json(json!({
                "extends": ["//"],
                "tasks": {
                    "omit-keys-task-with-deps": {}
                }
            })),
        ),
        (
            PackageName::from("override-values"),
            turbo_json(json!({
                "extends": ["//"],
                "tasks": {
                    "override-values-task-with-deps": { "dependsOn": [] }
                }
            })),
        ),
        (
            PackageName::from("cross-workspace"),
            turbo_json(json!({
                "extends": ["//"],
                "tasks": {
                    "cross-workspace-task": {
                        "dependsOn": ["blank-pkg#cross-workspace-underlying-task"]
                    }
                }
            })),
        ),
    ]
    .into_iter()
    .collect();
    let loader = TestTurboJsonLoader::new(turbo_jsons);

    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks(vec![
            Spanned::new(TaskName::from(
                "missing-workspace-config#missing-workspace-config-task-with-deps",
            )),
            Spanned::new(TaskName::from("omit-keys#omit-keys-task-with-deps")),
            Spanned::new(TaskName::from(
                "override-values#override-values-task-with-deps",
            )),
            Spanned::new(TaskName::from("cross-workspace#cross-workspace-task")),
        ])
        .with_workspaces(vec![
            PackageName::from("missing-workspace-config"),
            PackageName::from("omit-keys"),
            PackageName::from("override-values"),
            PackageName::from("cross-workspace"),
        ])
        .build()
        .unwrap();

    let override_def = task_definition(&engine, "override-values#override-values-task-with-deps");
    assert!(override_def.task_dependencies.is_empty());
    assert!(override_def.topological_dependencies.is_empty());
    assert_eq!(
        all_dependencies(&engine),
        deps! {
            "missing-workspace-config#missing-workspace-config-task-with-deps" => [
                "missing-workspace-config#missing-workspace-config-underlying-task",
                "blank-pkg#missing-workspace-config-underlying-topo-task"
            ],
            "missing-workspace-config#missing-workspace-config-underlying-task" => ["___ROOT___"],
            "blank-pkg#missing-workspace-config-underlying-topo-task" => ["___ROOT___"],
            "omit-keys#omit-keys-task-with-deps" => [
                "omit-keys#omit-keys-underlying-task",
                "blank-pkg#omit-keys-underlying-topo-task"
            ],
            "omit-keys#omit-keys-underlying-task" => ["___ROOT___"],
            "blank-pkg#omit-keys-underlying-topo-task" => ["___ROOT___"],
            "override-values#override-values-task-with-deps" => ["___ROOT___"],
            "cross-workspace#cross-workspace-task" => [
                "blank-pkg#cross-workspace-underlying-task"
            ],
            "blank-pkg#cross-workspace-underlying-task" => ["___ROOT___"]
        }
    );
}

// Test task-level extends: false to opt out of inherited tasks
