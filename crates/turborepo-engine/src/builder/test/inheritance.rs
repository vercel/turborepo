use super::*;

#[test]
fn test_task_extends_false_excludes_task() {
    // shared-config defines build and lint tasks
    // app extends shared-config but opts out of lint with extends: false
    let turbo_jsons = vec![
        (
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "test": {}
                }
            })),
        ),
        (
            PackageName::from("shared-config"),
            turbo_json(json!({
                "extends": ["//"],
                "tasks": {
                    "build": { "outputs": ["dist/**"] },
                    "lint": {}
                }
            })),
        ),
        (
            PackageName::from("app"),
            turbo_json(json!({
                "extends": ["//", "shared-config"],
                "tasks": {
                    "lint": { "extends": false }
                }
            })),
        ),
    ]
    .into_iter()
    .collect();

    let loader = TestTurboJsonLoader::new(turbo_jsons);

    // Collect tasks for app
    let mut tasks = HashSet::new();
    let mut visited = HashSet::new();
    collect_tasks_from_extends_chain(&loader, &PackageName::from("app"), &mut tasks, &mut visited)
        .unwrap();

    // app should have build (inherited) and test (from root) but NOT lint
    assert!(
        tasks.contains(&TaskName::from("build")),
        "Should have build from shared-config"
    );
    assert!(
        tasks.contains(&TaskName::from("test")),
        "Should have test from root"
    );
    assert!(
        !tasks.contains(&TaskName::from("lint")),
        "Should NOT have lint - excluded with extends: false"
    );
}

// Test task-level extends: false with local config creates fresh definition
#[test]
fn test_task_extends_false_with_config_creates_fresh_task() {
    // app has extends: false on build but provides its own config
    let turbo_jsons = vec![
        (
            PackageName::Root,
            turbo_json(json!({
                "tasks": {}
            })),
        ),
        (
            PackageName::from("shared-config"),
            turbo_json(json!({
                "extends": ["//"],
                "tasks": {
                    "build": { "outputs": ["dist/**"], "cache": true }
                }
            })),
        ),
        (
            PackageName::from("app"),
            turbo_json(json!({
                "extends": ["//", "shared-config"],
                "tasks": {
                    "build": {
                        "extends": false,
                        "outputs": ["custom-dist/**"],
                        "cache": false
                    }
                }
            })),
        ),
    ]
    .into_iter()
    .collect();

    let loader = TestTurboJsonLoader::new(turbo_jsons);

    // Collect tasks for app - should still have build (as fresh definition)
    let mut tasks = HashSet::new();
    let mut visited = HashSet::new();
    collect_tasks_from_extends_chain(&loader, &PackageName::from("app"), &mut tasks, &mut visited)
        .unwrap();

    assert!(
        tasks.contains(&TaskName::from("build")),
        "Should have build as a fresh definition"
    );
}

// Test error when extends: false is used on a task not in the extends chain
#[test]
fn test_task_extends_false_on_nonexistent_task_errors() {
    // app tries to opt out of "nonexistent" task that doesn't exist in chain
    let turbo_jsons = vec![
        (
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "build": {}
                }
            })),
        ),
        (
            PackageName::from("app"),
            turbo_json(json!({
                "extends": ["//"],
                "tasks": {
                    "nonexistent": { "extends": false }
                }
            })),
        ),
    ]
    .into_iter()
    .collect();

    let loader = TestTurboJsonLoader::new(turbo_jsons);

    // Should error because "nonexistent" is not in the extends chain
    let mut tasks = HashSet::new();
    let mut visited = HashSet::new();
    let result = collect_tasks_from_extends_chain(
        &loader,
        &PackageName::from("app"),
        &mut tasks,
        &mut visited,
    );

    assert!(
        result.is_err(),
        "Should error when extends: false is used on non-inherited task"
    );
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("nonexistent"),
        "Error should mention the task name"
    );
}

// Test that extends: true is a no-op (same as omitting the field)
#[test]
fn test_task_extends_true_is_noop() {
    let turbo_jsons = vec![
        (
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "build": { "cache": true }
                }
            })),
        ),
        (
            PackageName::from("app"),
            turbo_json(json!({
                "extends": ["//"],
                "tasks": {
                    "build": { "extends": true }
                }
            })),
        ),
    ]
    .into_iter()
    .collect();

    let loader = TestTurboJsonLoader::new(turbo_jsons);

    // Should have build task (inherited normally)
    let mut tasks = HashSet::new();
    let mut visited = HashSet::new();
    collect_tasks_from_extends_chain(&loader, &PackageName::from("app"), &mut tasks, &mut visited)
        .unwrap();

    assert!(
        tasks.contains(&TaskName::from("build")),
        "Should have build task with extends: true"
    );
}

// Test that has_task_definition_in_run returns false for tasks excluded via
// extends: false
#[test]
fn test_has_task_definition_returns_false_for_excluded_tasks() {
    let turbo_jsons = vec![
        (
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "build": {},
                    "lint": {}
                }
            })),
        ),
        (
            PackageName::from("app"),
            turbo_json(json!({
                "extends": ["//"],
                "tasks": {
                    "lint": { "extends": false }
                }
            })),
        ),
    ]
    .into_iter()
    .collect();

    let loader = TestTurboJsonLoader::new(turbo_jsons);

    // build should still be found (inherited from root)
    let task_name = TaskName::from("build");
    let task_id = TaskId::try_from("app#build").unwrap();
    let has_build = EngineBuilder::has_task_definition_in_run(
        &loader,
        &PackageName::from("app"),
        &task_name,
        &task_id,
    )
    .unwrap();
    assert!(has_build, "build should be found via extends chain");

    // lint should NOT be found (excluded via extends: false)
    let task_name = TaskName::from("lint");
    let task_id = TaskId::try_from("app#lint").unwrap();
    let has_lint = EngineBuilder::has_task_definition_in_run(
        &loader,
        &PackageName::from("app"),
        &task_name,
        &task_id,
    )
    .unwrap();
    assert!(
        !has_lint,
        "lint should NOT be found - excluded with extends: false"
    );
}

// Test that has_task_definition_in_run returns true for extends: false WITH
// config
#[test]
fn test_has_task_definition_returns_true_for_excluded_tasks_with_config() {
    let turbo_jsons = vec![
        (
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "build": { "cache": true }
                }
            })),
        ),
        (
            PackageName::from("app"),
            turbo_json(json!({
                "extends": ["//"],
                "tasks": {
                    "build": {
                        "extends": false,
                        "cache": false
                    }
                }
            })),
        ),
    ]
    .into_iter()
    .collect();

    let loader = TestTurboJsonLoader::new(turbo_jsons);

    // build should be found (has extends: false but also has config)
    let task_name = TaskName::from("build");
    let task_id = TaskId::try_from("app#build").unwrap();
    let has_build = EngineBuilder::has_task_definition_in_run(
        &loader,
        &PackageName::from("app"),
        &task_name,
        &task_id,
    )
    .unwrap();
    assert!(
        has_build,
        "build should be found - extends: false with config creates fresh definition"
    );
}

// ==================== Additional Test Coverage ====================
// The following tests cover gaps identified in the test coverage review

// Test multi-level task-level extends: A extends B, B has extends: false on
// task from C NOTE: The current implementation behavior is that `extends:
// false` only applies to the package where it's defined. If pkg-a extends
// pkg-b, and pkg-b excludes a task from pkg-c, pkg-a will still see the
// task because it collects from the full chain. This is intentional:
// exclusions are package-local, not propagated through the chain.
#[test]
fn test_multi_level_task_extends_false() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let _package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "pkg-a" => [],
            "pkg-b" => [],
            "pkg-c" => []
        },
    );

    // Chain: pkg-a extends pkg-b extends pkg-c extends root
    // pkg-c defines "lint" task
    // pkg-b excludes "lint" task via extends: false
    // pkg-a extends pkg-b
    //
    // Correct behavior: pkg-a should NOT see "lint" because exclusions propagate
    // through the extends chain. When pkg-b excludes "lint", all packages that
    // extend pkg-b (like pkg-a) will also not see "lint".
    let turbo_jsons = vec![
        (
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "build": {}
                }
            })),
        ),
        (
            PackageName::from("pkg-c"),
            turbo_json(json!({
                "extends": ["//"],
                "tasks": {
                    "lint": { "cache": true }
                }
            })),
        ),
        (
            PackageName::from("pkg-b"),
            turbo_json(json!({
                "extends": ["//", "pkg-c"],
                "tasks": {
                    "lint": { "extends": false }
                }
            })),
        ),
        (
            PackageName::from("pkg-a"),
            turbo_json(json!({
                "extends": ["//", "pkg-b"],
                "tasks": {}
            })),
        ),
    ]
    .into_iter()
    .collect();

    let loader = TestTurboJsonLoader::new(turbo_jsons);

    // pkg-a should NOT see lint because exclusions propagate through extends chain
    let mut tasks = HashSet::new();
    let mut visited = HashSet::new();
    collect_tasks_from_extends_chain(
        &loader,
        &PackageName::from("pkg-a"),
        &mut tasks,
        &mut visited,
    )
    .unwrap();

    assert!(
        tasks.contains(&TaskName::from("build")),
        "Should have build from root"
    );
    // lint is NOT visible to pkg-a because pkg-b's exclusion propagates
    assert!(
        !tasks.contains(&TaskName::from("lint")),
        "Should NOT have lint - exclusions propagate through extends chain"
    );

    // pkg-b itself should also NOT see lint
    let mut tasks_b = HashSet::new();
    let mut visited_b = HashSet::new();
    collect_tasks_from_extends_chain(
        &loader,
        &PackageName::from("pkg-b"),
        &mut tasks_b,
        &mut visited_b,
    )
    .unwrap();
    assert!(
        !tasks_b.contains(&TaskName::from("lint")),
        "pkg-b should NOT have lint - excluded locally"
    );
}

// Test that pkg-a can re-add an excluded task by defining it explicitly
#[test]
fn test_multi_level_task_extends_false_re_add() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let _package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "pkg-a" => [],
            "pkg-b" => [],
            "pkg-c" => []
        },
    );

    // Even though pkg-b excludes lint, pkg-a can re-add it by defining it
    // explicitly
    let turbo_jsons = vec![
        (
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "build": {}
                }
            })),
        ),
        (
            PackageName::from("pkg-c"),
            turbo_json(json!({
                "extends": ["//"],
                "tasks": {
                    "lint": { "cache": true }
                }
            })),
        ),
        (
            PackageName::from("pkg-b"),
            turbo_json(json!({
                "extends": ["//", "pkg-c"],
                "tasks": {
                    "lint": { "extends": false }
                }
            })),
        ),
        (
            PackageName::from("pkg-a"),
            turbo_json(json!({
                "extends": ["//", "pkg-b"],
                "tasks": {
                    // Explicitly define lint to re-add it (overrides pkg-b's exclusion)
                    "lint": { "cache": false }
                }
            })),
        ),
    ]
    .into_iter()
    .collect();

    let loader = TestTurboJsonLoader::new(turbo_jsons);

    // pkg-a should see lint because it explicitly defines it
    let mut tasks = HashSet::new();
    let mut visited = HashSet::new();
    collect_tasks_from_extends_chain(
        &loader,
        &PackageName::from("pkg-a"),
        &mut tasks,
        &mut visited,
    )
    .unwrap();

    assert!(
        tasks.contains(&TaskName::from("build")),
        "Should have build from root"
    );
    assert!(
        tasks.contains(&TaskName::from("lint")),
        "Should have lint - pkg-a re-added it explicitly"
    );
}

// Test multiple tasks excluded with extends: false in the same package
#[test]
fn test_multiple_tasks_excluded_with_extends_false() {
    let turbo_jsons = vec![
        (
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "build": {},
                    "lint": {},
                    "test": {},
                    "deploy": {}
                }
            })),
        ),
        (
            PackageName::from("app"),
            turbo_json(json!({
                "extends": ["//"],
                "tasks": {
                    "lint": { "extends": false },
                    "test": { "extends": false },
                    "custom": {}
                }
            })),
        ),
    ]
    .into_iter()
    .collect();

    let loader = TestTurboJsonLoader::new(turbo_jsons);

    let mut tasks = HashSet::new();
    let mut visited = HashSet::new();
    collect_tasks_from_extends_chain(&loader, &PackageName::from("app"), &mut tasks, &mut visited)
        .unwrap();

    // Should have build and deploy from root, custom from app
    assert!(
        tasks.contains(&TaskName::from("build")),
        "Should have build"
    );
    assert!(
        tasks.contains(&TaskName::from("deploy")),
        "Should have deploy"
    );
    assert!(
        tasks.contains(&TaskName::from("custom")),
        "Should have custom"
    );
    // lint and test should be excluded
    assert!(
        !tasks.contains(&TaskName::from("lint")),
        "Should NOT have lint"
    );
    assert!(
        !tasks.contains(&TaskName::from("test")),
        "Should NOT have test"
    );
}

// Test extends: false on the same task in multiple packages in the chain
#[test]
fn test_extends_false_same_task_multiple_packages() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let _package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "pkg-a" => [],
            "pkg-b" => []
        },
    );

    // Both pkg-a and pkg-b exclude "lint" task
    let turbo_jsons = vec![
        (
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "build": {},
                    "lint": {}
                }
            })),
        ),
        (
            PackageName::from("pkg-b"),
            turbo_json(json!({
                "extends": ["//"],
                "tasks": {
                    "lint": { "extends": false }
                }
            })),
        ),
        (
            PackageName::from("pkg-a"),
            turbo_json(json!({
                "extends": ["//", "pkg-b"],
                "tasks": {
                    "lint": { "extends": false }
                }
            })),
        ),
    ]
    .into_iter()
    .collect();

    let loader = TestTurboJsonLoader::new(turbo_jsons);

    let mut tasks = HashSet::new();
    let mut visited = HashSet::new();
    collect_tasks_from_extends_chain(
        &loader,
        &PackageName::from("pkg-a"),
        &mut tasks,
        &mut visited,
    )
    .unwrap();

    assert!(
        tasks.contains(&TaskName::from("build")),
        "Should have build"
    );
    assert!(
        !tasks.contains(&TaskName::from("lint")),
        "Should NOT have lint - excluded in both packages"
    );
}

// Test empty tasks objects in intermediate packages
#[test]
fn test_empty_tasks_in_intermediate_packages() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let _package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "pkg-a" => [],
            "pkg-b" => [],
            "pkg-c" => []
        },
    );

    // pkg-a extends pkg-b extends pkg-c extends root
    // pkg-b has empty tasks object
    let turbo_jsons = vec![
        (
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "root-task": {}
                }
            })),
        ),
        (
            PackageName::from("pkg-c"),
            turbo_json(json!({
                "extends": ["//"],
                "tasks": {
                    "c-task": {}
                }
            })),
        ),
        (
            PackageName::from("pkg-b"),
            turbo_json(json!({
                "extends": ["//", "pkg-c"],
                "tasks": {}
            })),
        ),
        (
            PackageName::from("pkg-a"),
            turbo_json(json!({
                "extends": ["//", "pkg-b"],
                "tasks": {
                    "a-task": {}
                }
            })),
        ),
    ]
    .into_iter()
    .collect();

    let loader = TestTurboJsonLoader::new(turbo_jsons);

    let mut tasks = HashSet::new();
    let mut visited = HashSet::new();
    collect_tasks_from_extends_chain(
        &loader,
        &PackageName::from("pkg-a"),
        &mut tasks,
        &mut visited,
    )
    .unwrap();

    // Should have all tasks despite empty tasks in pkg-b
    assert!(
        tasks.contains(&TaskName::from("root-task")),
        "Should have root-task from root"
    );
    assert!(
        tasks.contains(&TaskName::from("c-task")),
        "Should have c-task from pkg-c"
    );
    assert!(
        tasks.contains(&TaskName::from("a-task")),
        "Should have a-task from pkg-a"
    );
}

// Test extends: false with different config types (inputs, outputs, env)
#[test]
fn test_extends_false_with_various_configs() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "app" => []
        },
    );

    let turbo_jsons = vec![
        (
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "build": { "outputs": ["dist/**"], "cache": true },
                    "lint": {},
                    "test": {}
                }
            })),
        ),
        (
            PackageName::from("app"),
            turbo_json(json!({
                "extends": ["//"],
                "tasks": {
                    "build": {
                        "extends": false,
                        "outputs": ["custom-dist/**"],
                        "inputs": ["src/**"],
                        "env": ["NODE_ENV"]
                    },
                    "lint": {
                        "extends": false,
                        "persistent": true
                    },
                    "test": {
                        "extends": false,
                        "dependsOn": ["build"]
                    }
                }
            })),
        ),
    ]
    .into_iter()
    .collect();

    let loader = TestTurboJsonLoader::new(turbo_jsons);

    // All tasks should be found since they have config beyond extends
    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_future_flags(FutureFlags::default())
        .with_tasks(vec![
            Spanned::new(TaskName::from("build")),
            Spanned::new(TaskName::from("lint")),
            Spanned::new(TaskName::from("test")),
        ])
        .with_workspaces(vec![PackageName::from("app")])
        .build()
        .unwrap();

    // All tasks should be in the engine
    let expected = deps! {
        "app#build" => ["___ROOT___"],
        "app#lint" => ["___ROOT___"],
        "app#test" => ["app#build"]
    };
    assert_eq!(all_dependencies(&engine), expected);
}

// Test add_all_tasks with excluded tasks - full engine build
#[test]
fn test_add_all_tasks_with_excluded_tasks_full_build() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "app" => []
        },
    );

    let turbo_jsons = vec![
        (
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "build": {},
                    "lint": {},
                    "test": {}
                }
            })),
        ),
        (
            PackageName::from("app"),
            turbo_json(json!({
                "extends": ["//"],
                "tasks": {
                    "lint": { "extends": false }
                }
            })),
        ),
    ]
    .into_iter()
    .collect();

    let loader = TestTurboJsonLoader::new(turbo_jsons);

    // Use add_all_tasks mode
    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_future_flags(FutureFlags::default())
        .add_all_tasks()
        .with_workspaces(vec![PackageName::from("app")])
        .build()
        .unwrap();

    // Should have build and test, but NOT lint
    let task_ids: HashSet<_> = engine
        .task_lookup()
        .keys()
        .map(|id| id.to_string())
        .collect();

    assert!(task_ids.contains("app#build"), "Should have app#build");
    assert!(task_ids.contains("app#test"), "Should have app#test");
    assert!(
        !task_ids.contains("app#lint"),
        "Should NOT have app#lint - excluded"
    );
}

// Test that transit node pattern works with add_all_tasks (GitHub issue #11301)
// This tests the case where a root task like "transit" is defined without the
// //# prefix in turbo.json, but is used as a dependency from other tasks.
//
// The scenario: User has a turbo.json with:
//   "type-check": { "dependsOn": ["transit"] }
//   "transit": { "dependsOn": ["^transit"] }
//
// And a root package.json with a "type-check" script (but NOT a "transit"
// script). When devtools runs:
// 1. //#type-check is enabled as a root task (from package.json script)
// 2. Processing //#type-check adds //#transit as a dependency
// 3. //#transit should be allowed because it has a definition in turbo.json
#[test]
fn test_add_all_tasks_with_transit_node() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "app" => ["lib"],
            "lib" => []
        },
    );

    let turbo_jsons = vec![(
        PackageName::Root,
        turbo_json(json!({
            "tasks": {
                "type-check": { "dependsOn": ["transit"] },
                "transit": { "dependsOn": ["^transit"] }
            }
        })),
    )]
    .into_iter()
    .collect();

    let loader = TestTurboJsonLoader::new(turbo_jsons);

    // Simulate what devtools does:
    // - Collect task keys from turbo.json (type-check, transit - both without //#)
    // - Also add //#type-check because it's in root package.json scripts
    // The key is that //#type-check is enabled but //#transit is NOT explicitly
    // enabled - it only has a definition in turbo.json.
    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_root_tasks(vec![
            // This simulates having a "type-check" script in root package.json
            // Devtools adds it with //#  prefix
            TaskName::from("//#type-check"),
        ])
        .add_all_tasks()
        .with_workspaces(vec![
            PackageName::Root,
            PackageName::from("app"),
            PackageName::from("lib"),
        ])
        .build()
        .expect("Engine build should succeed with transit node pattern");

    let task_ids: HashSet<_> = engine
        .task_lookup()
        .keys()
        .map(|id| id.to_string())
        .collect();

    // Should have root tasks
    assert!(
        task_ids.contains("//#type-check"),
        "Should have //#type-check"
    );
    assert!(task_ids.contains("//#transit"), "Should have //#transit");

    // Should have workspace transit tasks from ^transit dependency
    assert!(task_ids.contains("app#transit"), "Should have app#transit");
    assert!(task_ids.contains("lib#transit"), "Should have lib#transit");

    // Verify the dependency graph structure
    let deps = all_dependencies(&engine);

    // //#type-check should depend on //#transit
    let type_check_deps = deps.get(&TaskId::try_from("//#type-check").unwrap());
    assert!(
        type_check_deps.is_some(),
        "//#type-check should have dependencies"
    );
    assert!(
        type_check_deps
            .unwrap()
            .contains(&TaskNode::Task(TaskId::try_from("//#transit").unwrap())),
        "//#type-check should depend on //#transit"
    );
}

// Test interaction with dependsOn and topological dependencies
#[test]
fn test_extends_false_with_dependson_topo() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "app" => ["lib"],
            "lib" => []
        },
    );

    let turbo_jsons = vec![
        (
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "build": { "dependsOn": ["^build"] },
                    "lint": { "dependsOn": ["build"] }
                }
            })),
        ),
        (
            PackageName::from("app"),
            turbo_json(json!({
                "extends": ["//"],
                "tasks": {
                    "build": {
                        "extends": false,
                        "dependsOn": ["^build", "prepare"]
                    },
                    "prepare": {}
                }
            })),
        ),
    ]
    .into_iter()
    .collect();

    let loader = TestTurboJsonLoader::new(turbo_jsons);

    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_future_flags(FutureFlags::default())
        .with_tasks(Some(Spanned::new(TaskName::from("build"))))
        .with_workspaces(vec![PackageName::from("app")])
        .build()
        .unwrap();

    // app#build should depend on lib#build (^build) and app#prepare (fresh
    // definition)
    let expected = deps! {
        "app#build" => ["lib#build", "app#prepare"],
        "lib#build" => ["___ROOT___"],
        "app#prepare" => ["___ROOT___"]
    };
    assert_eq!(all_dependencies(&engine), expected);
}

// Test order of extends array affecting task resolution
#[test]
fn test_extends_order_affects_resolution() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let _package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "app" => [],
            "config-a" => [],
            "config-b" => []
        },
    );

    // config-a and config-b both define same task with different configs
    // Order in extends should determine which is used
    let turbo_jsons = vec![
        (
            PackageName::Root,
            turbo_json(json!({
                "tasks": {}
            })),
        ),
        (
            PackageName::from("config-a"),
            turbo_json(json!({
                "extends": ["//"],
                "tasks": {
                    "build": { "outputs": ["dist-a/**"] }
                }
            })),
        ),
        (
            PackageName::from("config-b"),
            turbo_json(json!({
                "extends": ["//"],
                "tasks": {
                    "build": { "outputs": ["dist-b/**"] }
                }
            })),
        ),
        (
            PackageName::from("app"),
            turbo_json(json!({
                "extends": ["//", "config-a", "config-b"],
                "tasks": {}
            })),
        ),
    ]
    .into_iter()
    .collect();

    let loader = TestTurboJsonLoader::new(turbo_jsons);

    // Tasks should be discovered from both - deduplication happens by task name
    let mut tasks = HashSet::new();
    let mut visited = HashSet::new();
    collect_tasks_from_extends_chain(&loader, &PackageName::from("app"), &mut tasks, &mut visited)
        .unwrap();

    // Should have build task (deduplicated)
    assert!(
        tasks.contains(&TaskName::from("build")),
        "Should have build task"
    );
    assert_eq!(tasks.len(), 1, "Should only have one unique task");
}

// Test that extends: false requires the task to exist in the chain (error case
// verification)
#[test]
fn test_extends_false_error_message_quality() {
    let turbo_jsons = vec![
        (
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "build": {},
                    "test": {}
                }
            })),
        ),
        (
            PackageName::from("app"),
            turbo_json(json!({
                "extends": ["//"],
                "tasks": {
                    "nonexistent-task": { "extends": false }
                }
            })),
        ),
    ]
    .into_iter()
    .collect();

    let loader = TestTurboJsonLoader::new(turbo_jsons);

    let mut tasks = HashSet::new();
    let mut visited = HashSet::new();
    let result = collect_tasks_from_extends_chain(
        &loader,
        &PackageName::from("app"),
        &mut tasks,
        &mut visited,
    );

    assert!(result.is_err());
    let err = result.unwrap_err();
    let err_string = err.to_string();

    // Error should mention the task name
    assert!(
        err_string.contains("nonexistent-task"),
        "Error should mention the task name: {}",
        err_string
    );
}

// Test extends: true mixed with extends: false in same package
#[test]
fn test_extends_true_and_false_mixed() {
    let turbo_jsons = vec![
        (
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "build": { "cache": true },
                    "lint": { "cache": true },
                    "test": { "cache": true }
                }
            })),
        ),
        (
            PackageName::from("app"),
            turbo_json(json!({
                "extends": ["//"],
                "tasks": {
                    "build": { "extends": true },
                    "lint": { "extends": false },
                    "test": {}
                }
            })),
        ),
    ]
    .into_iter()
    .collect();

    let loader = TestTurboJsonLoader::new(turbo_jsons);

    let mut tasks = HashSet::new();
    let mut visited = HashSet::new();
    collect_tasks_from_extends_chain(&loader, &PackageName::from("app"), &mut tasks, &mut visited)
        .unwrap();

    // build should be inherited (extends: true)
    // lint should be excluded (extends: false)
    // test should be inherited (no extends field = normal inheritance)
    assert!(
        tasks.contains(&TaskName::from("build")),
        "Should have build"
    );
    assert!(tasks.contains(&TaskName::from("test")), "Should have test");
    assert!(
        !tasks.contains(&TaskName::from("lint")),
        "Should NOT have lint"
    );
}

// Test task discovery when workspace has no turbo.json but extends from a
// package that DOES have one
#[test]
fn test_workspace_without_turbo_json_with_extends_in_root() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let _package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "shared-config" => [],
            "app-with-config" => [],
            "app-without-config" => []
        },
    );

    // shared-config defines tasks
    // app-with-config extends shared-config
    // app-without-config has NO turbo.json (should fallback to root)
    let turbo_jsons = vec![
        (
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "root-build": {}
                }
            })),
        ),
        (
            PackageName::from("shared-config"),
            turbo_json(json!({
                "extends": ["//"],
                "tasks": {
                    "shared-build": {}
                }
            })),
        ),
        (
            PackageName::from("app-with-config"),
            turbo_json(json!({
                "extends": ["//", "shared-config"],
                "tasks": {}
            })),
        ),
        // Note: app-without-config has NO turbo.json entry
    ]
    .into_iter()
    .collect();

    let loader = TestTurboJsonLoader::new(turbo_jsons);

    // app-with-config should have both root-build and shared-build
    let mut tasks1 = HashSet::new();
    let mut visited1 = HashSet::new();
    collect_tasks_from_extends_chain(
        &loader,
        &PackageName::from("app-with-config"),
        &mut tasks1,
        &mut visited1,
    )
    .unwrap();

    assert!(tasks1.contains(&TaskName::from("root-build")));
    assert!(tasks1.contains(&TaskName::from("shared-build")));

    // app-without-config should fallback to root (only root-build)
    let mut tasks2 = HashSet::new();
    let mut visited2 = HashSet::new();
    collect_tasks_from_extends_chain(
        &loader,
        &PackageName::from("app-without-config"),
        &mut tasks2,
        &mut visited2,
    )
    .unwrap();

    assert!(tasks2.contains(&TaskName::from("root-build")));
    // Should NOT have shared-build since app-without-config doesn't extend
    // shared-config
    assert!(!tasks2.contains(&TaskName::from("shared-build")));
}

// Test partial exclusion - only exclude task for specific package via extends:
// false
#[test]
fn test_partial_exclusion_specific_package() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let _package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "app1" => [],
            "app2" => []
        },
    );

    let turbo_jsons = vec![
        (
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "build": {},
                    "lint": {}
                }
            })),
        ),
        (
            PackageName::from("app1"),
            turbo_json(json!({
                "extends": ["//"],
                "tasks": {
                    "lint": { "extends": false }
                }
            })),
        ),
        (
            PackageName::from("app2"),
            turbo_json(json!({
                "extends": ["//"],
                "tasks": {}
            })),
        ),
    ]
    .into_iter()
    .collect();

    let loader = TestTurboJsonLoader::new(turbo_jsons);

    // app1 should NOT have lint
    let mut tasks1 = HashSet::new();
    let mut visited1 = HashSet::new();
    collect_tasks_from_extends_chain(
        &loader,
        &PackageName::from("app1"),
        &mut tasks1,
        &mut visited1,
    )
    .unwrap();
    assert!(tasks1.contains(&TaskName::from("build")));
    assert!(!tasks1.contains(&TaskName::from("lint")));

    // app2 SHOULD have lint (exclusion is package-specific)
    let mut tasks2 = HashSet::new();
    let mut visited2 = HashSet::new();
    collect_tasks_from_extends_chain(
        &loader,
        &PackageName::from("app2"),
        &mut tasks2,
        &mut visited2,
    )
    .unwrap();
    assert!(tasks2.contains(&TaskName::from("build")));
    assert!(tasks2.contains(&TaskName::from("lint")));
}

// Test that engine building with multiple workspaces handles exclusions
// correctly
#[test]
fn test_engine_multiple_workspaces_with_exclusions() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "app1" => [],
            "app2" => []
        },
    );

    let turbo_jsons = vec![
        (
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "build": {}
                }
            })),
        ),
        (
            PackageName::from("app1"),
            turbo_json(json!({
                "extends": ["//"],
                "tasks": {
                    "build": { "extends": false }
                }
            })),
        ),
        (
            PackageName::from("app2"),
            turbo_json(json!({
                "extends": ["//"],
                "tasks": {}
            })),
        ),
    ]
    .into_iter()
    .collect();

    let loader = TestTurboJsonLoader::new(turbo_jsons);

    // Build with both workspaces
    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_future_flags(FutureFlags::default())
        .with_tasks(Some(Spanned::new(TaskName::from("build"))))
        .with_workspaces(vec![PackageName::from("app1"), PackageName::from("app2")])
        .build()
        .unwrap();

    // Only app2#build should be in the engine (app1 excluded it)
    let task_ids: HashSet<_> = engine
        .task_lookup()
        .keys()
        .map(|id| id.to_string())
        .collect();

    assert!(
        !task_ids.contains("app1#build"),
        "Should NOT have app1#build - excluded"
    );
    assert!(task_ids.contains("app2#build"), "Should have app2#build");
}

// Test cyclic extends with task exclusion doesn't cause issues
#[test]
fn test_cyclic_extends_with_task_exclusion() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let _package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "pkg-a" => [],
            "pkg-b" => []
        },
    );

    // Create a cycle with task exclusions
    let turbo_jsons = vec![
        (
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "build": {},
                    "lint": {}
                }
            })),
        ),
        (
            PackageName::from("pkg-a"),
            turbo_json(json!({
                "extends": ["//", "pkg-b"],
                "tasks": {
                    "lint": { "extends": false }
                }
            })),
        ),
        (
            PackageName::from("pkg-b"),
            turbo_json(json!({
                "extends": ["//", "pkg-a"],
                "tasks": {
                    "custom-b": {}
                }
            })),
        ),
    ]
    .into_iter()
    .collect();

    let loader = TestTurboJsonLoader::new(turbo_jsons);

    // Should handle cycle gracefully even with task exclusion
    let mut tasks = HashSet::new();
    let mut visited = HashSet::new();
    collect_tasks_from_extends_chain(
        &loader,
        &PackageName::from("pkg-a"),
        &mut tasks,
        &mut visited,
    )
    .unwrap();

    // Should have build and custom-b, but NOT lint
    assert!(
        tasks.contains(&TaskName::from("build")),
        "Should have build"
    );
    assert!(
        tasks.contains(&TaskName::from("custom-b")),
        "Should have custom-b from pkg-b"
    );
    assert!(
        !tasks.contains(&TaskName::from("lint")),
        "Should NOT have lint - excluded"
    );
}

// Test that task_definition_chain correctly handles extends: false in
// intermediate packages. This ensures that when a shared-config package
// uses `extends: false` for a task, packages extending from it will
// use the shared-config's definition, not the root's.
#[test]
fn test_task_definition_chain_with_extends_false_in_intermediate() {
    // Scenario:
    // - Root turbo.json: defines build: { cache: true, outputs: ["dist/**"] }
    // - shared-config/turbo.json: extends root, defines build: { extends: false,
    //   cache: false }
    // - app/turbo.json: extends shared-config, empty tasks
    //
    // Expected: app#build should use shared-config's cache: false, NOT root's
    // cache: true
    let turbo_jsons = vec![
        (
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "build": { "cache": true, "outputs": ["dist/**"] }
                }
            })),
        ),
        (
            PackageName::from("shared-config"),
            turbo_json(json!({
                "extends": ["//"],
                "tasks": {
                    "build": {
                        "extends": false,
                        "cache": false
                    }
                }
            })),
        ),
        (
            PackageName::from("app"),
            turbo_json(json!({
                "extends": ["//", "shared-config"],
                "tasks": {}
            })),
        ),
    ]
    .into_iter()
    .collect();

    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "shared-config" => [],
            "app" => []
        },
    );

    let loader = TestTurboJsonLoader::new(turbo_jsons);
    let engine_builder = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_future_flags(FutureFlags::default());

    // Verify task_definition_chain gets definitions from shared-config, not root
    let task_name = TaskName::from("build");
    let task_id = TaskId::try_from("app#build").unwrap();
    let task_id_spanned = Spanned::new(task_id);
    let definitions = engine_builder
        .task_definition_chain(&loader, &task_id_spanned, &task_name)
        .unwrap();

    assert!(
        !definitions.is_empty(),
        "task_definition_chain should return definitions for app#build"
    );

    // Should use shared-config's cache: false (not root's cache: true)
    // The first definition in the chain should be from shared-config
    if let Some(first_def) = definitions.first() {
        assert_eq!(
            first_def.cache.as_ref().map(|c| *c.as_inner()),
            Some(false),
            "Should use shared-config cache: false, not root cache: true"
        );
    }

    // There should only be one definition (shared-config's), not two
    assert_eq!(
        definitions.len(),
        1,
        "Should only have one definition from shared-config, not root + shared-config"
    );
}
