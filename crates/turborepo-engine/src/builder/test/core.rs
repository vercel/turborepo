use super::*;

#[test]
fn test_default_engine() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "a" => [],
            "b" => [],
            "c" => ["a", "b"]
        },
    );
    let turbo_jsons = vec![(
        PackageName::Root,
        turbo_json(json!({
            "tasks": {
                "test": { "dependsOn": ["^build", "prepare"] },
                "build": { "dependsOn": ["^build", "prepare"] },
                "prepare": {},
                "side-quest": { "dependsOn": ["prepare"] },
            }
        })),
    )]
    .into_iter()
    .collect();
    let loader = TestTurboJsonLoader::new(turbo_jsons);
    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks(Some(Spanned::new(TaskName::from("test"))))
        .with_workspaces(vec![
            PackageName::from("a"),
            PackageName::from("b"),
            PackageName::from("c"),
        ])
        .build()
        .unwrap();

    let expected = deps! {
        "a#test" => ["a#prepare"],
        "a#build" => ["a#prepare"],
        "a#prepare" => ["___ROOT___"],
        "b#test" => ["b#prepare"],
        "b#build" => ["b#prepare"],
        "b#prepare" => ["___ROOT___"],
        "c#prepare" => ["___ROOT___"],
        "c#test" => ["a#build", "b#build", "c#prepare"]
    };
    assert_eq!(all_dependencies(&engine), expected);
}

#[test]
fn test_dependencies_on_unspecified_packages() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    // app1 -> libA
    //              \
    //                > libB -> libD
    //              /
    //       app2 <
    //              \ libC
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "app1" => ["libA"],
            "app2" => ["libB", "libC"],
            "libA" => ["libB"],
            "libB" => ["libD"],
            "libC" => [],
            "libD" => []
        },
    );
    let turbo_jsons = vec![(
        PackageName::Root,
        turbo_json(json!({
            "tasks": {
                "test": { "dependsOn": ["^build"] },
                "build": { "dependsOn": ["^build"] },
            }
        })),
    )]
    .into_iter()
    .collect();
    let loader = TestTurboJsonLoader::new(turbo_jsons);
    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks(Some(Spanned::new(TaskName::from("test"))))
        .with_workspaces(vec![PackageName::from("app2")])
        .build()
        .unwrap();

    let expected = deps! {
        "app2#test" => ["libB#build", "libC#build"],
        "libB#build" => ["libD#build"],
        "libC#build" => ["___ROOT___"],
        "libD#build" => ["___ROOT___"]
    };
    assert_eq!(all_dependencies(&engine), expected);
}

#[test]
fn test_run_package_task() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "app1" => ["libA"],
            "libA" => []
        },
    );
    let turbo_jsons = vec![(
        PackageName::Root,
        turbo_json(json!({
            "tasks": {
                "build": { "dependsOn": ["^build"] },
                "app1#special": { "dependsOn": ["^build"] },
            }
        })),
    )]
    .into_iter()
    .collect();
    let loader = TestTurboJsonLoader::new(turbo_jsons);
    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks(Some(Spanned::new(TaskName::from("special"))))
        .with_workspaces(vec![PackageName::from("app1"), PackageName::from("libA")])
        .build()
        .unwrap();

    let expected = deps! {
        "app1#special" => ["libA#build"],
        "libA#build" => ["___ROOT___"]
    };
    assert_eq!(all_dependencies(&engine), expected);
}

#[test]
fn test_include_root_tasks() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "app1" => ["libA"],
            "libA" => []
        },
    );
    let turbo_jsons = vec![(
        PackageName::Root,
        turbo_json(json!({
            "tasks": {
                "build": { "dependsOn": ["^build"] },
                "test": { "dependsOn": ["^build"] },
                "//#test": {},
            }
        })),
    )]
    .into_iter()
    .collect();
    let loader = TestTurboJsonLoader::new(turbo_jsons);
    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks(vec![
            Spanned::new(TaskName::from("build")),
            Spanned::new(TaskName::from("test")),
        ])
        .with_workspaces(vec![
            PackageName::Root,
            PackageName::from("app1"),
            PackageName::from("libA"),
        ])
        .with_root_tasks(vec![
            TaskName::from("//#test"),
            TaskName::from("build"),
            TaskName::from("test"),
        ])
        .build()
        .unwrap();

    let expected = deps! {
        "//#test" => ["___ROOT___"],
        "app1#build" => ["libA#build"],
        "app1#test" => ["libA#build"],
        "libA#build" => ["___ROOT___"],
        "libA#test" => ["___ROOT___"]
    };
    assert_eq!(all_dependencies(&engine), expected);
}

#[test]
fn test_depend_on_root_task() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "app1" => ["libA"],
            "libA" => []
        },
    );
    let turbo_jsons = vec![(
        PackageName::Root,
        turbo_json(json!({
            "tasks": {
                "build": { "dependsOn": ["^build"] },
                "libA#build": { "dependsOn": ["//#root-task"] },
                "//#root-task": {},
            }
        })),
    )]
    .into_iter()
    .collect();
    let loader = TestTurboJsonLoader::new(turbo_jsons);
    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks(Some(Spanned::new(TaskName::from("build"))))
        .with_workspaces(vec![PackageName::from("app1")])
        .with_root_tasks(vec![
            TaskName::from("//#root-task"),
            TaskName::from("libA#build"),
            TaskName::from("build"),
        ])
        .build()
        .unwrap();

    let expected = deps! {
        "//#root-task" => ["___ROOT___"],
        "app1#build" => ["libA#build"],
        "libA#build" => ["//#root-task"]
    };
    assert_eq!(all_dependencies(&engine), expected);
}

#[test]
fn test_root_task_depends_on_workspace_task() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "lib-a" => []
        },
    );
    let turbo_jsons = vec![(
        PackageName::Root,
        turbo_json(json!({
            "tasks": {
                "//#mytask": { "dependsOn": ["lib-a#build"] },
                "build": {}
            }
        })),
    )]
    .into_iter()
    .collect();
    let loader = TestTurboJsonLoader::new(turbo_jsons);
    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks(Some(Spanned::new(TaskName::from("mytask"))))
        .with_workspaces(vec![PackageName::Root, PackageName::from("lib-a")])
        .with_root_tasks(vec![TaskName::from("//#mytask")])
        .build()
        .unwrap();

    let expected = deps! {
        "//#mytask" => ["lib-a#build"],
        "lib-a#build" => ["___ROOT___"]
    };
    assert_eq!(all_dependencies(&engine), expected);
}

#[test]
fn test_depend_on_missing_task() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "app1" => ["libA"],
            "libA" => []
        },
    );
    let turbo_jsons = vec![(
        PackageName::Root,
        turbo_json(json!({
            "tasks": {
                "build": { "dependsOn": ["^build"] },
                "libA#build": { "dependsOn": ["//#root-task"] },
            }
        })),
    )]
    .into_iter()
    .collect();
    let loader = TestTurboJsonLoader::new(turbo_jsons);
    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks(Some(Spanned::new(TaskName::from("build"))))
        .with_workspaces(vec![PackageName::from("app1")])
        .with_root_tasks(vec![TaskName::from("libA#build"), TaskName::from("build")])
        .build();

    let err = engine.unwrap_err();
    assert!(matches!(err, BuilderError::MissingRootTaskInTurboJson(_)));
    let message = err.to_string();
    assert!(
        message.contains("//#root-task requires an entry in turbo.json")
            && message.contains("because it is a task declared in the root package.json"),
        "unexpected missing root task message: {message}"
    );
}

#[test]
fn test_depend_on_missing_package_task_errors() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "app-a" => [],
            "app-b" => []
        },
    );
    let turbo_jsons = vec![(
        PackageName::Root,
        turbo_json(json!({
            "tasks": {
                "build2": { "dependsOn": ["app-a#custom"] }
            }
        })),
    )]
    .into_iter()
    .collect();
    let loader = TestTurboJsonLoader::new(turbo_jsons);
    let result = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks(Some(Spanned::new(TaskName::from("build2"))))
        .with_workspaces(vec![PackageName::from("app-b")])
        .build();

    let err = result.unwrap_err();
    assert!(matches!(err, BuilderError::MissingPackageTask(_)));
    assert!(
        err.to_string()
            .contains("Could not find \"app-a#custom\" in root turbo.json"),
        "unexpected missing package task error: {err}"
    );
}

#[test]
fn test_depend_on_missing_package_errors() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "app-b" => []
        },
    );
    let turbo_jsons = vec![(
        PackageName::Root,
        turbo_json(json!({
            "tasks": {
                "build3": { "dependsOn": ["unknown#custom"] }
            }
        })),
    )]
    .into_iter()
    .collect();
    let loader = TestTurboJsonLoader::new(turbo_jsons);
    let result = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks(Some(Spanned::new(TaskName::from("build3"))))
        .with_workspaces(vec![PackageName::from("app-b")])
        .build();

    let err = result.unwrap_err();
    assert!(matches!(err, BuilderError::MissingPackageFromTask(_)));
    assert!(
        err.to_string()
            .contains("Could not find package \"unknown\" referenced by task"),
        "unexpected missing package error: {err}"
    );
}

#[test]
fn test_depend_on_multiple_package_tasks() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "app1" => ["libA"],
            "libA" => []
        },
    );
    let turbo_jsons = vec![(
        PackageName::Root,
        turbo_json(json!({
            "tasks": {
                "libA#build": { "dependsOn": ["app1#compile", "app1#test"] },
                "build": { "dependsOn": ["^build"] },
                "compile": {},
                "test": {}
            }
        })),
    )]
    .into_iter()
    .collect();
    let loader = TestTurboJsonLoader::new(turbo_jsons);
    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks(Some(Spanned::new(TaskName::from("build"))))
        .with_workspaces(vec![PackageName::from("app1")])
        .with_root_tasks(vec![
            TaskName::from("libA#build"),
            TaskName::from("build"),
            TaskName::from("compile"),
        ])
        .build()
        .unwrap();

    let expected = deps! {
        "app1#compile" => ["___ROOT___"],
        "app1#test" => ["___ROOT___"],
        "app1#build" => ["libA#build"],
        "libA#build" => ["app1#compile", "app1#test"]
    };
    assert_eq!(all_dependencies(&engine), expected);
}

#[test]
fn test_package_specific_task_overrides_depends_on() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "workspace-a" => [],
            "workspace-b" => []
        },
    );
    let turbo_jsons = vec![(
        PackageName::Root,
        turbo_json(json!({
            "tasks": {
                "build": { "dependsOn": ["generate"] },
                "generate": {},
                "workspace-b#build": { "dependsOn": [] }
            }
        })),
    )]
    .into_iter()
    .collect();
    let loader = TestTurboJsonLoader::new(turbo_jsons);
    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks(Some(Spanned::new(TaskName::from("build"))))
        .with_workspaces(vec![
            PackageName::from("workspace-a"),
            PackageName::from("workspace-b"),
        ])
        .build()
        .unwrap();

    let expected = deps! {
        "workspace-a#build" => ["workspace-a#generate"],
        "workspace-a#generate" => ["___ROOT___"],
        "workspace-b#build" => ["___ROOT___"]
    };
    assert_eq!(all_dependencies(&engine), expected);
}

#[test]
fn test_explicit_task_self_dependency_errors() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "app" => []
        },
    );
    let turbo_jsons = vec![(
        PackageName::Root,
        turbo_json(json!({
            "tasks": {
                "build4": { "dependsOn": ["build4"] }
            }
        })),
    )]
    .into_iter()
    .collect();
    let loader = TestTurboJsonLoader::new(turbo_jsons);
    let result = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks(Some(Spanned::new(TaskName::from("build4"))))
        .with_workspaces(vec![PackageName::from("app")])
        .build();

    assert!(
        matches!(
            result,
            Err(BuilderError::Graph(
                turborepo_graph_utils::Error::SelfDependency(ref task)
            )) if task == "app#build4"
        ),
        "expected explicit task self-dependency error, got: {result:?}"
    );
}

#[test]
fn test_depends_on_disabled_root_task() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "app1" => ["libA"],
            "libA" => []
        },
    );
    let turbo_jsons = vec![(
        PackageName::Root,
        turbo_json(json!({
            "tasks": {
                "build": { "dependsOn": ["^build"] },
                "foo": {},
                "libA#build": { "dependsOn": ["//#foo"] }
            }
        })),
    )]
    .into_iter()
    .collect();
    let loader = TestTurboJsonLoader::new(turbo_jsons);
    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks(Some(Spanned::new(TaskName::from("build"))))
        .with_workspaces(vec![PackageName::from("app1")])
        .with_root_tasks(vec![
            TaskName::from("libA#build"),
            TaskName::from("build"),
            TaskName::from("foo"),
        ])
        .build();

    assert!(
        matches!(engine, Err(BuilderError::MissingRootTaskInTurboJson(_))),
        "Expected MissingRootTaskInTurboJson error"
    );
}

#[test]
fn test_engine_tasks_only() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "a" => [],
            "b" => [],
            "c" => ["a", "b"]
        },
    );
    let turbo_jsons = vec![(
        PackageName::Root,
        turbo_json(json!({
            "tasks": {
                "build": { "dependsOn": ["^build", "prepare"] },
                "test": { "dependsOn": ["^build", "prepare"] },
                "prepare": {},
            }
        })),
    )]
    .into_iter()
    .collect();
    let loader = TestTurboJsonLoader::new(turbo_jsons);
    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks_only(true)
        .with_tasks(Some(Spanned::new(TaskName::from("test"))))
        .with_workspaces(vec![
            PackageName::from("a"),
            PackageName::from("b"),
            PackageName::from("c"),
        ])
        .with_root_tasks(vec![
            TaskName::from("build"),
            TaskName::from("test"),
            TaskName::from("prepare"),
        ])
        .build()
        .unwrap();

    let expected = deps! {
        "a#test" => ["___ROOT___"],
        "b#test" => ["___ROOT___"],
        "c#test" => ["___ROOT___"]
    };
    assert_eq!(all_dependencies(&engine), expected);
}

#[test]
fn test_engine_tasks_only_package_deps() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "a" => [],
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
        .with_tasks_only(true)
        .with_tasks(Some(Spanned::new(TaskName::from("build"))))
        .with_workspaces(vec![PackageName::from("b")])
        .with_root_tasks(vec![TaskName::from("build")])
        .build()
        .unwrap();

    // With task only we shouldn't do package tasks dependencies either
    let expected = deps! {
        "b#build" => ["___ROOT___"]
    };
    assert_eq!(all_dependencies(&engine), expected);
}

#[test]
fn test_engine_tasks_only_task_dep() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "a" => [],
            "b" => []
        },
    );
    let turbo_jsons = vec![(
        PackageName::Root,
        turbo_json(json!({
            "tasks": {
                "a#build": { },
                "b#build": { "dependsOn": ["a#build"] }
            }
        })),
    )]
    .into_iter()
    .collect();
    let loader = TestTurboJsonLoader::new(turbo_jsons);
    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks_only(true)
        .with_tasks(Some(Spanned::new(TaskName::from("build"))))
        .with_workspaces(vec![PackageName::from("b")])
        .with_root_tasks(vec![TaskName::from("build")])
        .build()
        .unwrap();

    // With task only we shouldn't do package tasks dependencies either
    let expected = deps! {
        "b#build" => ["___ROOT___"]
    };
    assert_eq!(all_dependencies(&engine), expected);
}

// Note: test_validate_task_name has been moved to turborepo-engine crate
// See: crates/turborepo-engine/src/validate.rs

#[test]
fn test_run_package_task_exact() {
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
    let turbo_jsons = vec![
        (
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "build": { "dependsOn": ["^build"] },
                    "special": { "dependsOn": ["^build"] },
                }
            })),
        ),
        (
            PackageName::from("app2"),
            turbo_json(json!({
                "extends": ["//"],
                "tasks": {
                    "another": { "dependsOn": ["^build"] },
                }
            })),
        ),
    ]
    .into_iter()
    .collect();
    let loader = TestTurboJsonLoader::new(turbo_jsons);
    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks(vec![
            Spanned::new(TaskName::from("app1#special")),
            Spanned::new(TaskName::from("app2#another")),
        ])
        .with_workspaces(vec![PackageName::from("app1"), PackageName::from("app2")])
        .build()
        .unwrap();

    let expected = deps! {
        "app1#special" => ["libA#build"],
        "app2#another" => ["libA#build"],
        "libA#build" => ["___ROOT___"]
    };
    assert_eq!(all_dependencies(&engine), expected);
}

#[test]
fn test_with_task() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "web" => [],
            "api" => []
        },
    );
    let turbo_jsons = vec![(PackageName::Root, {
        turbo_json(json!({
            "tasks": {
                "web#dev": { "persistent": true, "with": ["api#serve"] },
                "api#serve": { "persistent": true }
            }
        }))
    })]
    .into_iter()
    .collect();
    let loader = TestTurboJsonLoader::new(turbo_jsons);
    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks(Some(Spanned::new(TaskName::from("dev"))))
        .with_workspaces(vec![PackageName::from("web")])
        .build()
        .unwrap();

    let expected = deps! {
        "web#dev" => ["___ROOT___"],
        "api#serve" => ["___ROOT___"]
    };
    assert_eq!(all_dependencies(&engine), expected);
}

#[test]
fn test_run_package_task_exact_error() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "app1" => ["libA"],
            "libA" => []
        },
    );
    let turbo_jsons = vec![
        (
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "build": { "dependsOn": ["^build"] },
                }
            })),
        ),
        (
            PackageName::from("app1"),
            turbo_json(json!({
                "extends": ["//"],
                "tasks": {
                    "another": { "dependsOn": ["^build"] },
                }
            })),
        ),
    ]
    .into_iter()
    .collect();
    let loader = TestTurboJsonLoader::new(turbo_jsons);
    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks(vec![Spanned::new(TaskName::from("app1#special"))])
        .with_workspaces(vec![PackageName::from("app1")])
        .build();
    assert!(engine.is_err());
    let report = miette::Report::new(engine.unwrap_err());
    let mut msg = String::new();
    miette::JSONReportHandler::new()
        .render_report(&mut msg, report.as_ref())
        .unwrap();
    assert_json_snapshot!(msg);

    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks(vec![Spanned::new(TaskName::from("app1#another"))])
        .with_workspaces(vec![PackageName::from("libA")])
        .build()
        .unwrap();
    assert_eq!(engine.tasks().collect::<Vec<_>>(), &[&TaskNode::Root]);
}

#[test]
fn test_run_package_task_invalid_package() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "app1" => ["libA"],
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
    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks(vec![Spanned::new(TaskName::from("app2#bad-task"))])
        .with_workspaces(vec![PackageName::from("app1"), PackageName::from("libA")])
        .build();
    assert!(engine.is_err());
    let report = miette::Report::new(engine.unwrap_err());
    let mut msg = String::new();
    miette::NarratableReportHandler::new()
        .render_report(&mut msg, report.as_ref())
        .unwrap();
    assert_snapshot!(msg);
}

#[test]
fn test_filter_removes_task_def() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "app1" => ["libA"],
            "libA" => []
        },
    );
    let turbo_jsons = vec![
        (
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "build": { "dependsOn": ["^build"] },
                }
            })),
        ),
        (
            PackageName::from("app1"),
            turbo_json(json!({
                "tasks": {
                    "app1-only": {},
                }
            })),
        ),
    ]
    .into_iter()
    .collect();
    let loader = TestTurboJsonLoader::new(turbo_jsons);
    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks(vec![Spanned::new(TaskName::from("app1-only"))])
        .with_workspaces(vec![PackageName::from("libA")])
        .build()
        .unwrap();
    assert_eq!(
        engine.tasks().collect::<Vec<_>>(),
        &[&TaskNode::Root],
        "only the root task node should be present"
    );
}

#[test]
fn test_path_to_root() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "app1" => ["libA"],
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
    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false);
    assert_eq!(
        engine
            .path_to_root(&TaskId::new("//", "build"))
            .unwrap()
            .as_str(),
        "."
    );
    // libA is located at packages/libA
    assert_eq!(
        engine
            .path_to_root(&TaskId::new("libA", "build"))
            .unwrap()
            .as_str(),
        "../.."
    );
}

fn stub_io_engine(
    task_definition: serde_json::Value,
    outputs: turborepo_repository::toolchain::DerivedOutputs,
    task: &str,
    pass_through_args: Vec<String>,
    requested_tasks: Vec<String>,
) -> StubIOEngineResult {
    let repo_root_dir = TempDir::with_prefix("stub-io").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let seen = Arc::new(Mutex::new(HashMap::new()));
    let toolchain = Arc::new(StubIOToolchain {
        repo_root: repo_root.clone(),
        outputs,
        environment: vec!["STUB_LAYOUT"],
        seen: seen.clone(),
    });
    let package_graph = stub_io_package_graph(&repo_root, toolchain);
    let loader = TestTurboJsonLoader::new(
        vec![(
            PackageName::Root,
            turbo_json(json!({ "tasks": task_definition })),
        )]
        .into_iter()
        .collect(),
    );
    let environments = HashMap::from([(
        ToolchainId::new("stub-io"),
        turborepo_repository::toolchain::TaskIOEnvironment::new(HashMap::from([(
            "STUB_LAYOUT".to_string(),
            "layout-value".to_string(),
        )])),
    )]);
    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_tasks(Some(Spanned::new(TaskName::from(task).into_owned())))
        .with_workspaces(vec![PackageName::from("app")])
        .with_task_io_context(pass_through_args, requested_tasks, environments)
        .build()
        .unwrap();
    (engine, seen)
}

#[test]
fn test_task_io_args_align_with_execution_for_dependencies() {
    let (_engine, seen) = stub_io_engine(
        json!({
            "test": { "dependsOn": ["^build"] },
            "build": {}
        }),
        DerivedOutputs::Resolved(Vec::new()),
        "test",
        vec!["--requested".to_string()],
        vec!["test".to_string()],
    );
    let seen = seen.lock().unwrap();
    assert_eq!(
        seen.get("app#test"),
        Some(&SeenTaskIO {
            args: Some(vec!["--requested".to_string()]),
            layout_env: Some("layout-value".to_string()),
        })
    );
    assert_eq!(
        seen.get("lib#build"),
        Some(&SeenTaskIO {
            args: None,
            layout_env: Some("layout-value".to_string()),
        })
    );
}

#[test]
fn test_unavailable_outputs_respect_merged_task_configuration() {
    for (definition, expected_cache, expected_outputs) in [
        (json!({ "build": {} }), false, Vec::<String>::new()),
        (json!({ "build": { "cache": true } }), true, Vec::new()),
        (json!({ "build": { "cache": false } }), false, Vec::new()),
        (
            json!({ "build": { "outputs": ["configured/**"] } }),
            true,
            vec!["configured/**".to_string()],
        ),
        (json!({ "build": { "outputs": [] } }), true, Vec::new()),
    ] {
        let (engine, _) = stub_io_engine(
            definition,
            DerivedOutputs::Unavailable,
            "build",
            Vec::new(),
            vec!["build".to_string()],
        );
        let task = engine
            .task_definition(&TaskId::new("app", "build"))
            .unwrap();
        assert_eq!(task.cache, expected_cache);
        assert_eq!(task.outputs.inclusions, expected_outputs);
        assert!(task.env.contains(&"STUB_LAYOUT".to_string()));
    }
}
