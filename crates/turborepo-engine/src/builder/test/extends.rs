use super::*;

#[test]
fn test_cyclic_extends() {
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

    // Create a self-referencing cycle: Root extends itself
    let turbo_jsons = vec![
        (
            PackageName::Root,
            turbo_json(json!({
                "extends": ["//"],  // Root extending itself creates a cycle
                "tasks": {
                    "build": {}
                }
            })),
        ),
        (
            PackageName::from("app1"),
            turbo_json(json!({
                "extends": ["//"],
                "tasks": {}
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
    let engine_result = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks(Some(Spanned::new(TaskName::from("build"))))
        .with_workspaces(vec![PackageName::from("app1")])
        .build();

    assert!(engine_result.is_err());
    if let Err(BuilderError::CyclicExtends(boxed)) = engine_result {
        let CyclicExtends { cycle, .. } = *boxed;
        // The cycle should contain root (//) since it's a self-reference
        assert!(cycle.contains(&"//".to_string()));
        // Should have at least 2 entries to show the cycle (// -> //)
        assert!(cycle.len() >= 2);
    } else {
        panic!("Expected CyclicExtends error, got {:?}", engine_result);
    }
}

// Test that tasks are inherited from non-root extends even when child has no
// tasks key
#[test]
fn test_extends_inherits_tasks_from_non_root_package() {
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

    // Setup:
    // - shared-config defines a "build" task
    // - app extends from root and shared-config but has NO tasks key
    // - app should still be able to run the "build" task inherited from
    //   shared-config
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
                    "build": { "inputs": ["src/**"] }
                }
            })),
        ),
        (
            PackageName::from("app"),
            // app extends from root and shared-config but has NO tasks defined
            turbo_json(json!({
                "extends": ["//", "shared-config"]
            })),
        ),
    ]
    .into_iter()
    .collect();

    let loader = TestTurboJsonLoader::new(turbo_jsons);

    // Verify that "app" can find the "build" task inherited from "shared-config"
    let task_name = TaskName::from("build");
    let task_id = TaskId::try_from("app#build").unwrap();
    let has_def = EngineBuilder::has_task_definition_in_run(
        &loader,
        &PackageName::from("app"),
        &task_name,
        &task_id,
    )
    .unwrap();
    assert!(
        has_def,
        "app should inherit 'build' task from shared-config via extends"
    );

    // Also verify the engine can be built with this task
    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_future_flags(FutureFlags::default())
        .with_tasks(Some(Spanned::new(TaskName::from("build"))))
        .with_workspaces(vec![PackageName::from("app")])
        .build()
        .unwrap();

    // The engine should contain the app#build task
    let expected = deps! {
        "app#build" => ["___ROOT___"]
    };
    assert_eq!(all_dependencies(&engine), expected);
}

// Test that tasks are discovered from non-root extends when using add_all_tasks
#[test]
fn test_add_all_tasks_discovers_extended_tasks() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let _package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "shared-config" => [],
            "app" => []
        },
    );

    // Setup:
    // - root has "test" task
    // - shared-config has "build" task
    // - app extends from shared-config but has no tasks
    // - When using add_all_tasks, "build" should be discovered for app
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
                    "build": {}
                }
            })),
        ),
        (
            PackageName::from("app"),
            turbo_json(json!({
                "extends": ["//", "shared-config"]
            })),
        ),
    ]
    .into_iter()
    .collect();

    let loader = TestTurboJsonLoader::new(turbo_jsons);

    // Test collect_tasks_from_extends_chain
    let mut tasks = HashSet::new();
    let mut visited = HashSet::new();
    collect_tasks_from_extends_chain(&loader, &PackageName::from("app"), &mut tasks, &mut visited)
        .unwrap();

    // app should have discovered "build" from shared-config and "test" from root
    assert!(
        tasks.contains(&TaskName::from("build")),
        "Should discover 'build' task from shared-config"
    );
    assert!(
        tasks.contains(&TaskName::from("test")),
        "Should discover 'test' task from root"
    );
}

// Test A→B→A cycle handling (gracefully handled via visited set)
#[test]
fn test_cyclic_extends_between_packages_graceful() {
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

    // Create a cycle: pkg-a extends pkg-b, pkg-b extends pkg-a
    // Note: Both extend root first to satisfy validation
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
            PackageName::from("pkg-a"),
            turbo_json(json!({
                "extends": ["//", "pkg-b"],
                "tasks": {
                    "task-a": {}
                }
            })),
        ),
        (
            PackageName::from("pkg-b"),
            turbo_json(json!({
                "extends": ["//", "pkg-a"],
                "tasks": {
                    "task-b": {}
                }
            })),
        ),
    ]
    .into_iter()
    .collect();

    let loader = TestTurboJsonLoader::new(turbo_jsons);

    // The cycle is handled gracefully via the visited set - it doesn't error,
    // it just stops recursion when it encounters a visited package.
    // This test verifies that the cycle doesn't cause infinite recursion
    // and that we still collect all reachable tasks.
    let mut tasks = HashSet::new();
    let mut visited = HashSet::new();
    collect_tasks_from_extends_chain(
        &loader,
        &PackageName::from("pkg-a"),
        &mut tasks,
        &mut visited,
    )
    .unwrap();

    // Should have collected tasks from pkg-a, pkg-b, and root (despite the cycle)
    assert!(
        tasks.contains(&TaskName::from("task-a")),
        "Should have task-a"
    );
    assert!(
        tasks.contains(&TaskName::from("task-b")),
        "Should have task-b"
    );
    assert!(
        tasks.contains(&TaskName::from("root-task")),
        "Should have root-task"
    );

    // Also verify has_task_definition_in_run handles the cycle gracefully
    let task_name = TaskName::from("task-b");
    let task_id = TaskId::try_from("pkg-a#task-b").unwrap();
    let has_def = EngineBuilder::has_task_definition_in_run(
        &loader,
        &PackageName::from("pkg-a"),
        &task_name,
        &task_id,
    )
    .unwrap();
    assert!(has_def, "Should find task-b via extends chain");
}

// Test deep extends chain: A extends B extends C extends D extends root
#[test]
fn test_deep_extends_chain() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "pkg-a" => [],
            "pkg-b" => [],
            "pkg-c" => [],
            "pkg-d" => []
        },
    );

    // Create a deep chain: pkg-a -> pkg-b -> pkg-c -> pkg-d -> root
    // Each level adds a unique task
    // Note: Each package must extend root first to satisfy validation
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
            PackageName::from("pkg-d"),
            turbo_json(json!({
                "extends": ["//"],
                "tasks": {
                    "task-d": {}
                }
            })),
        ),
        (
            PackageName::from("pkg-c"),
            turbo_json(json!({
                "extends": ["//", "pkg-d"],
                "tasks": {
                    "task-c": {}
                }
            })),
        ),
        (
            PackageName::from("pkg-b"),
            turbo_json(json!({
                "extends": ["//", "pkg-c"],
                "tasks": {
                    "task-b": {}
                }
            })),
        ),
        (
            PackageName::from("pkg-a"),
            turbo_json(json!({
                "extends": ["//", "pkg-b"],
                "tasks": {
                    "task-a": {}
                }
            })),
        ),
    ]
    .into_iter()
    .collect();

    let loader = TestTurboJsonLoader::new(turbo_jsons);

    // Test that pkg-a can discover all tasks from the entire chain
    let mut tasks = HashSet::new();
    let mut visited = HashSet::new();
    collect_tasks_from_extends_chain(
        &loader,
        &PackageName::from("pkg-a"),
        &mut tasks,
        &mut visited,
    )
    .unwrap();

    // pkg-a should have all tasks from the chain
    assert!(
        tasks.contains(&TaskName::from("task-a")),
        "Should have task-a"
    );
    assert!(
        tasks.contains(&TaskName::from("task-b")),
        "Should have task-b from pkg-b"
    );
    assert!(
        tasks.contains(&TaskName::from("task-c")),
        "Should have task-c from pkg-c"
    );
    assert!(
        tasks.contains(&TaskName::from("task-d")),
        "Should have task-d from pkg-d"
    );
    assert!(
        tasks.contains(&TaskName::from("root-task")),
        "Should have root-task from root"
    );

    // Also verify has_task_definition_in_run works for deep chain
    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_future_flags(FutureFlags::default())
        .with_tasks(Some(Spanned::new(TaskName::from("task-d"))))
        .with_workspaces(vec![PackageName::from("pkg-a")])
        .build()
        .unwrap();

    let expected = deps! {
        "pkg-a#task-d" => ["___ROOT___"]
    };
    assert_eq!(all_dependencies(&engine), expected);
}

// Test diamond inheritance: app extends [base1, base2], both base1 and base2
// extend root
#[test]
fn test_diamond_inheritance_deduplication() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "base1" => [],
            "base2" => [],
            "app" => []
        },
    );

    // Diamond pattern:
    //        app
    //       /   \
    //    base1  base2
    //       \   /
    //        root
    // Both base1 and base2 define "build" task, app should only get it once
    let turbo_jsons = vec![
        (
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "root-task": {},
                    "build": {}  // Also defined in root
                }
            })),
        ),
        (
            PackageName::from("base1"),
            turbo_json(json!({
                "extends": ["//"],
                "tasks": {
                    "build": {},  // Same task name as base2
                    "base1-only": {}
                }
            })),
        ),
        (
            PackageName::from("base2"),
            turbo_json(json!({
                "extends": ["//"],
                "tasks": {
                    "build": {},  // Same task name as base1
                    "base2-only": {}
                }
            })),
        ),
        (
            PackageName::from("app"),
            turbo_json(json!({
                "extends": ["//", "base1", "base2"],
                "tasks": {}
            })),
        ),
    ]
    .into_iter()
    .collect();

    let loader = TestTurboJsonLoader::new(turbo_jsons);

    // Test that tasks are deduplicated
    let mut tasks = HashSet::new();
    let mut visited = HashSet::new();
    collect_tasks_from_extends_chain(&loader, &PackageName::from("app"), &mut tasks, &mut visited)
        .unwrap();

    // Should have all unique tasks
    assert!(
        tasks.contains(&TaskName::from("build")),
        "Should have build"
    );
    assert!(
        tasks.contains(&TaskName::from("root-task")),
        "Should have root-task"
    );
    assert!(
        tasks.contains(&TaskName::from("base1-only")),
        "Should have base1-only"
    );
    assert!(
        tasks.contains(&TaskName::from("base2-only")),
        "Should have base2-only"
    );

    // Verify count - build should only appear once due to HashSet deduplication
    assert_eq!(tasks.len(), 4, "Should have exactly 4 unique tasks");

    // Also verify the engine builds successfully
    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_future_flags(FutureFlags::default())
        .with_tasks(Some(Spanned::new(TaskName::from("build"))))
        .with_workspaces(vec![PackageName::from("app")])
        .build()
        .unwrap();

    let expected = deps! {
        "app#build" => ["___ROOT___"]
    };
    assert_eq!(all_dependencies(&engine), expected);
}

// Test that workspace without turbo.json falls back to root
#[test]
fn test_missing_workspace_turbo_json_fallback() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "app-with-config" => [],
            "app-without-config" => []
        },
    );

    // Only root and app-with-config have turbo.json
    // app-without-config has no turbo.json and should fall back to root
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
            PackageName::from("app-with-config"),
            turbo_json(json!({
                "extends": ["//"],
                "tasks": {
                    "custom": {}
                }
            })),
        ),
        // Note: app-without-config has NO turbo.json entry
    ]
    .into_iter()
    .collect();

    let loader = TestTurboJsonLoader::new(turbo_jsons);

    // Test collect_tasks_from_extends_chain for workspace without turbo.json
    let mut tasks = HashSet::new();
    let mut visited = HashSet::new();
    collect_tasks_from_extends_chain(
        &loader,
        &PackageName::from("app-without-config"),
        &mut tasks,
        &mut visited,
    )
    .unwrap();

    // Should fall back to root and get root's tasks
    assert!(
        tasks.contains(&TaskName::from("build")),
        "Should have build from root fallback"
    );
    assert!(
        tasks.contains(&TaskName::from("test")),
        "Should have test from root fallback"
    );

    // Test has_task_definition_in_run for workspace without turbo.json
    let task_name = TaskName::from("build");
    let task_id = TaskId::try_from("app-without-config#build").unwrap();
    let has_def = EngineBuilder::has_task_definition_in_run(
        &loader,
        &PackageName::from("app-without-config"),
        &task_name,
        &task_id,
    )
    .unwrap();
    assert!(
        has_def,
        "app-without-config should find 'build' task via root fallback"
    );

    // Verify engine builds correctly for workspace without turbo.json
    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks(Some(Spanned::new(TaskName::from("build"))))
        .with_workspaces(vec![PackageName::from("app-without-config")])
        .build()
        .unwrap();

    let expected = deps! {
        "app-without-config#build" => ["___ROOT___"]
    };
    assert_eq!(all_dependencies(&engine), expected);
}
