use super::*;

#[test]
fn jit_task_graph_keeps_topological_and_same_package_dependencies() {
    let repo_root_dir = TempDir::new().unwrap();
    let repo_root = AbsoluteSystemPathBuf::try_from(repo_root_dir.path()).unwrap();
    let package_graph = mock_package_graph(
        &repo_root,
        package_jsons! {
            repo_root,
            "my-app" => ["util"],
            "util" => []
        },
    );
    let loader = TestTurboJsonLoader::new(HashMap::from([(
        PackageName::Root,
        turbo_json(json!({
            "tasks": {
                "codegen": {
                    "dependsOn": ["^build"],
                    "cache": false
                },
                "build": {
                    "dependsOn": ["^build", "codegen"],
                    "inputs": [
                        "$TURBO_DEFAULT$",
                        "!src/generated/**",
                        {
                            "mode": "jit",
                            "globs": ["src/generated/**"]
                        }
                    ],
                    "outputs": [".output/**"]
                }
            }
        })),
    )]));
    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_workspaces(vec![PackageName::from("my-app"), PackageName::from("util")])
        .with_tasks(Some(Spanned::new(TaskName::from("build"))))
        .build()
        .unwrap();

    assert_eq!(
        all_dependencies(&engine),
        deps! {
            "my-app#build" => ["my-app#codegen", "util#build"],
            "my-app#codegen" => ["util#build"],
            "util#build" => ["util#codegen"],
            "util#codegen" => ["___ROOT___"]
        }
    );
    let build = task_definition(&engine, "my-app#build");
    assert!(build.inputs.has_jit_inputs());
    assert_eq!(build.inputs.jit_globs, ["src/generated/**"]);
    assert!(!task_definition(&engine, "my-app#codegen").cache);
}
