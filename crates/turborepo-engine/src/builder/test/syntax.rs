use super::*;

#[test]
fn test_package_task_syntax_filters_workspaces() {
    // When using "app1#build" syntax with all workspaces, the engine should
    // produce the same graph as when only "app1" is in the workspace list.
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "app1" => ["libA"],
            "app2" => ["libA"],
            "libA" => []
        },
    );
    let turbo_jsons = vec![(
        PackageName::Root,
        turbo_json(json!({
            "tasks": {
                "build": { "dependsOn": ["^build"] },
            }
        })),
    )]
    .into_iter()
    .collect();
    let loader = TestTurboJsonLoader::new(turbo_jsons);

    // Simulate `turbo run app1#build` without --filter (all workspaces passed in)
    let engine_all_workspaces = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks(Some(Spanned::new(TaskName::from("app1#build"))))
        .with_workspaces(vec![
            PackageName::from("app1"),
            PackageName::from("app2"),
            PackageName::from("libA"),
        ])
        .build()
        .unwrap();

    // Simulate `turbo run build --filter=app1` (only app1 in workspaces)
    let engine_filtered = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks(Some(Spanned::new(TaskName::from("build"))))
        .with_workspaces(vec![PackageName::from("app1")])
        .build()
        .unwrap();

    let expected = deps! {
        "app1#build" => ["libA#build"],
        "libA#build" => ["___ROOT___"]
    };
    assert_eq!(all_dependencies(&engine_all_workspaces), expected);
    assert_eq!(all_dependencies(&engine_filtered), expected);
}

#[test]
fn test_package_task_syntax_mixed_with_plain_task() {
    // "turbo run app1#build lint" should run app1#build only for app1,
    // but lint for all workspaces.
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "app1" => ["libA"],
            "app2" => [],
            "libA" => []
        },
    );
    let turbo_jsons = vec![(
        PackageName::Root,
        turbo_json(json!({
            "tasks": {
                "build": { "dependsOn": ["^build"] },
                "lint": {},
            }
        })),
    )]
    .into_iter()
    .collect();
    let loader = TestTurboJsonLoader::new(turbo_jsons);

    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks(vec![
            Spanned::new(TaskName::from("app1#build")),
            Spanned::new(TaskName::from("lint")),
        ])
        .with_workspaces(vec![
            PackageName::from("app1"),
            PackageName::from("app2"),
            PackageName::from("libA"),
        ])
        .build()
        .unwrap();

    let expected = deps! {
        "app1#build" => ["libA#build"],
        "libA#build" => ["___ROOT___"],
        "app1#lint" => ["___ROOT___"],
        "app2#lint" => ["___ROOT___"],
        "libA#lint" => ["___ROOT___"]
    };
    assert_eq!(all_dependencies(&engine), expected);
}

#[test]
fn test_multiple_package_tasks_target_different_packages() {
    // "turbo run app1#build app2#test" with all workspaces should only
    // process each package-qualified task for its target package.
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "app1" => ["libA"],
            "app2" => ["libA"],
            "libA" => []
        },
    );
    let turbo_jsons = vec![(
        PackageName::Root,
        turbo_json(json!({
            "tasks": {
                "build": { "dependsOn": ["^build"] },
                "test": { "dependsOn": ["^build"] },
            }
        })),
    )]
    .into_iter()
    .collect();
    let loader = TestTurboJsonLoader::new(turbo_jsons);

    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks(vec![
            Spanned::new(TaskName::from("app1#build")),
            Spanned::new(TaskName::from("app2#test")),
        ])
        .with_workspaces(vec![
            PackageName::from("app1"),
            PackageName::from("app2"),
            PackageName::from("libA"),
        ])
        .build()
        .unwrap();

    let expected = deps! {
        "app1#build" => ["libA#build"],
        "app2#test" => ["libA#build"],
        "libA#build" => ["___ROOT___"]
    };
    assert_eq!(all_dependencies(&engine), expected);
}

#[test]
fn test_root_task_with_package_task_syntax_and_all_workspaces() {
    // When mixing a root-enabled task ("rootlint") with a package-qualified
    // task ("app1#build") and all workspaces, the root task should only run
    // on root, and the package-qualified task only on its target package.
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "app1" => ["libA"],
            "app2" => [],
            "libA" => []
        },
    );
    let turbo_jsons = vec![(
        PackageName::Root,
        turbo_json(json!({
            "tasks": {
                "build": { "dependsOn": ["^build"] },
                "//#rootlint": {},
            }
        })),
    )]
    .into_iter()
    .collect();
    let loader = TestTurboJsonLoader::new(turbo_jsons);

    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks(vec![
            Spanned::new(TaskName::from("app1#build")),
            Spanned::new(TaskName::from("rootlint")),
        ])
        .with_root_tasks(vec![TaskName::from("//#rootlint")])
        .with_workspaces(vec![
            PackageName::Root,
            PackageName::from("app1"),
            PackageName::from("app2"),
            PackageName::from("libA"),
        ])
        .build()
        .unwrap();

    let expected = deps! {
        "app1#build" => ["libA#build"],
        "libA#build" => ["___ROOT___"],
        "//#rootlint" => ["___ROOT___"]
    };
    assert_eq!(all_dependencies(&engine), expected);
}

#[test]
fn test_root_task_with_double_slash_hash_cli_syntax() {
    // Regression test for https://github.com/vercel/turborepo/issues/12239
    // Running `turbo run //#root-level-call` (with the //#  prefix on the CLI task)
    // must execute the root task. Previously, the root_enabled_tasks comparison
    // failed because it compared the package-qualified TaskName against the
    // stripped version stored in root_enabled_tasks.
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
    let turbo_jsons = vec![(
        PackageName::Root,
        turbo_json(json!({
            "tasks": {
                "//#root-level-call": {},
            }
        })),
    )]
    .into_iter()
    .collect();
    let loader = TestTurboJsonLoader::new(turbo_jsons);

    // The key difference from the existing test: the CLI task uses
    // "//#root-level-call" syntax (with the //#  prefix), not just
    // "root-level-call".
    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks(Some(Spanned::new(TaskName::from("//#root-level-call"))))
        .with_root_tasks(vec![TaskName::from("//#root-level-call")])
        .with_workspaces(vec![PackageName::Root])
        .build()
        .unwrap();

    let expected = deps! {
        "//#root-level-call" => ["___ROOT___"]
    };
    assert_eq!(all_dependencies(&engine), expected);
}

#[test]
fn test_root_task_double_slash_hash_mixed_with_package_task() {
    // Verify that `//#roottask` syntax works alongside `package#task` syntax.
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "app1" => ["libA"],
            "app2" => [],
            "libA" => []
        },
    );
    let turbo_jsons = vec![(
        PackageName::Root,
        turbo_json(json!({
            "tasks": {
                "build": { "dependsOn": ["^build"] },
                "//#rootlint": {},
            }
        })),
    )]
    .into_iter()
    .collect();
    let loader = TestTurboJsonLoader::new(turbo_jsons);

    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks(vec![
            Spanned::new(TaskName::from("app1#build")),
            Spanned::new(TaskName::from("//#rootlint")),
        ])
        .with_root_tasks(vec![TaskName::from("//#rootlint")])
        .with_workspaces(vec![
            PackageName::Root,
            PackageName::from("app1"),
            PackageName::from("app2"),
            PackageName::from("libA"),
        ])
        .build()
        .unwrap();

    let expected = deps! {
        "app1#build" => ["libA#build"],
        "libA#build" => ["___ROOT___"],
        "//#rootlint" => ["___ROOT___"]
    };
    assert_eq!(all_dependencies(&engine), expected);
}

#[test]
fn test_empty_workspaces_with_invalid_task_errors() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "a" => []
        },
    );
    let turbo_jsons = vec![(
        PackageName::Root,
        turbo_json(json!({
            "tasks": {
                "build": {},
            }
        })),
    )]
    .into_iter()
    .collect();
    let loader = TestTurboJsonLoader::new(turbo_jsons);

    let result = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks(Some(Spanned::new(TaskName::from("foobarbaz"))))
        .with_workspaces(vec![])
        .build();

    assert!(
        matches!(result, Err(BuilderError::MissingTasks(_))),
        "expected MissingTasks error for non-existent task with empty workspaces, got: {result:?}"
    );
}

// --- Cyclic package graph tests (see #2559) ---

#[test]
fn test_cyclic_package_graph_without_task_cycle_succeeds() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "a" => ["b"],
            "b" => ["a"]
        },
    );
    let turbo_jsons = vec![(
        PackageName::Root,
        turbo_json(json!({
            "tasks": {
                "lint": {},
                "build": { "dependsOn": ["lint"] },
            }
        })),
    )]
    .into_iter()
    .collect();
    let loader = TestTurboJsonLoader::new(turbo_jsons);
    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks(Some(Spanned::new(TaskName::from("build"))))
        .with_workspaces(vec![PackageName::from("a"), PackageName::from("b")])
        .build()
        .unwrap();

    assert_eq!(
        all_dependencies(&engine),
        deps! {
            "a#build" => ["a#lint"],
            "a#lint" => ["___ROOT___"],
            "b#build" => ["b#lint"],
            "b#lint" => ["___ROOT___"]
        }
    );
}

#[test]
fn test_cyclic_package_graph_filtered_workspace_succeeds() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "pkg-a" => ["pkg-b"],
            "pkg-b" => ["pkg-a"]
        },
    );
    let turbo_jsons = vec![(
        PackageName::Root,
        turbo_json(json!({
            "tasks": {
                "lint": {}
            }
        })),
    )]
    .into_iter()
    .collect();
    let loader = TestTurboJsonLoader::new(turbo_jsons);
    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks(Some(Spanned::new(TaskName::from("lint"))))
        .with_workspaces(vec![PackageName::from("pkg-a")])
        .build()
        .unwrap();

    assert_eq!(
        all_dependencies(&engine),
        deps! {
            "pkg-a#lint" => ["___ROOT___"]
        }
    );
}

#[test]
fn test_cyclic_package_graph_with_topo_deps_produces_task_cycle_error() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "a" => ["b"],
            "b" => ["a"]
        },
    );
    let turbo_jsons = vec![(
        PackageName::Root,
        turbo_json(json!({
            "tasks": {
                "build": { "dependsOn": ["^build"] },
            }
        })),
    )]
    .into_iter()
    .collect();
    let loader = TestTurboJsonLoader::new(turbo_jsons);
    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks(Some(Spanned::new(TaskName::from("build"))))
        .with_workspaces(vec![PackageName::from("a"), PackageName::from("b")])
        .build();

    assert!(
        matches!(engine, Err(BuilderError::Graph(..))),
        "topological deps on cyclic package graph should produce task graph cycle error: \
         {engine:?}"
    );
}

#[test]
fn test_three_node_cycle_with_topo_deps_produces_task_cycle_error() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "a" => ["b"],
            "b" => ["c"],
            "c" => ["a"]
        },
    );
    let turbo_jsons = vec![(
        PackageName::Root,
        turbo_json(json!({
            "tasks": {
                "build": { "dependsOn": ["^build"] },
            }
        })),
    )]
    .into_iter()
    .collect();
    let loader = TestTurboJsonLoader::new(turbo_jsons);
    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks(Some(Spanned::new(TaskName::from("build"))))
        .with_workspaces(vec![
            PackageName::from("a"),
            PackageName::from("b"),
            PackageName::from("c"),
        ])
        .build();

    assert!(
        matches!(engine, Err(BuilderError::Graph(..))),
        "3-node cycle with topological deps should produce task graph cycle error: {engine:?}"
    );
}

#[test]
fn test_self_dependency_without_topo_deps_succeeds() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "a" => ["a"]
        },
    );
    let turbo_jsons = vec![(
        PackageName::Root,
        turbo_json(json!({
            "tasks": {
                "lint": {},
                "build": { "dependsOn": ["lint"] },
            }
        })),
    )]
    .into_iter()
    .collect();
    let loader = TestTurboJsonLoader::new(turbo_jsons);
    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks(Some(Spanned::new(TaskName::from("build"))))
        .with_workspaces(vec![PackageName::from("a")])
        .build()
        .unwrap();

    assert_eq!(
        all_dependencies(&engine),
        deps! {
            "a#build" => ["a#lint"],
            "a#lint" => ["___ROOT___"]
        }
    );
}

#[test]
fn test_self_dependency_with_topo_deps_produces_error() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "a" => ["a"]
        },
    );
    let turbo_jsons = vec![(
        PackageName::Root,
        turbo_json(json!({
            "tasks": {
                "build": { "dependsOn": ["^build"] },
            }
        })),
    )]
    .into_iter()
    .collect();
    let loader = TestTurboJsonLoader::new(turbo_jsons);
    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks(Some(Spanned::new(TaskName::from("build"))))
        .with_workspaces(vec![PackageName::from("a")])
        .build();

    assert!(
        matches!(engine, Err(BuilderError::Graph(..))),
        "self-dependency with topological deps should produce task graph error: {engine:?}"
    );
}

#[test]
fn test_empty_workspaces_with_valid_task_succeeds() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "a" => []
        },
    );
    let turbo_jsons = vec![(
        PackageName::Root,
        turbo_json(json!({
            "tasks": {
                "build": {},
            }
        })),
    )]
    .into_iter()
    .collect();
    let loader = TestTurboJsonLoader::new(turbo_jsons);

    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks(Some(Spanned::new(TaskName::from("build"))))
        .with_workspaces(vec![])
        .build()
        .unwrap();

    assert!(
        engine.task_ids().count() == 0,
        "expected empty engine when workspaces is empty but task is valid"
    );
}
