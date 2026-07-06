use turborepo_repository::uv::{UvPackageKind, WORKSPACE_PACKAGE_NAME};

use super::*;

/// Build a package graph over a uv workspace fixture in a tempdir: `app`
/// (packaged) depends on `lib-a` (packaged) alongside `virt` (virtual).
/// Discovery parses the manifests directly; no uv binary is required.
fn uv_package_graph(repo_root: &turbopath::AbsoluteSystemPath) -> PackageGraph {
    let write = |rel: &[&str], contents: &str| {
        let path = repo_root.join_components(rel);
        std::fs::create_dir_all(path.parent().unwrap().as_std_path()).unwrap();
        std::fs::write(path.as_std_path(), contents).unwrap();
    };
    write(
        &["pyproject.toml"],
        "[project]\nname = \"root-project\"\nversion = \"0.1.0\"\n\n[tool.uv.workspace]\nmembers \
         = [\"packages/*\"]\n",
    );
    write(
        &["packages", "lib-a", "pyproject.toml"],
        "[project]\nname = \"lib-a\"\nversion = \"0.1.0\"\ndependencies = \
         []\n\n[build-system]\nrequires = [\"uv_build\"]\nbuild-backend = \"uv_build\"\n",
    );
    write(
        &["packages", "app", "pyproject.toml"],
        "[project]\nname = \"app\"\nversion = \"0.1.0\"\ndependencies = \
         [\"lib-a\"]\n\n[build-system]\nrequires = [\"uv_build\"]\nbuild-backend = \
         \"uv_build\"\n\n[tool.uv.sources]\nlib-a = { workspace = true }\n",
    );
    write(
        &["packages", "virt", "pyproject.toml"],
        "[project]\nname = \"virt\"\nversion = \"0.1.0\"\ndependencies = []\n",
    );

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(
        PackageGraph::builder(repo_root, PackageJson::default())
            .with_package_discovery(MockDiscovery)
            .with_lockfile(Some(Box::new(MockLockfile)))
            .with_package_jsons(Some(HashMap::new()))
            .with_uv(true)
            .build(),
    )
    .unwrap()
}

fn uv_engine(
    repo_root: &turbopath::AbsoluteSystemPath,
    package_graph: &PackageGraph,
    loader: &TestTurboJsonLoader,
    task: &'static str,
    global_deps: Vec<String>,
) -> crate::Engine<Built, TaskDefinition> {
    EngineBuilder::new(repo_root, package_graph, loader, false)
        .with_tasks(Some(Spanned::new(TaskName::from(task))))
        .with_workspaces(vec![
            PackageName::from("app"),
            PackageName::from("lib-a"),
            PackageName::from("virt"),
            PackageName::from(WORKSPACE_PACKAGE_NAME),
        ])
        .with_global_deps(global_deps)
        .build()
        .unwrap()
}

fn root_turbo_jsons(tasks: serde_json::Value) -> HashMap<PackageName, TurboJson> {
    vec![(PackageName::Root, turbo_json(json!({ "tasks": tasks })))]
        .into_iter()
        .collect()
}

#[test]
fn test_uv_packaged_build_task_wiring() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = uv_package_graph(&repo_root);
    assert_eq!(
        package_graph
            .package_info(&PackageName::from("app"))
            .unwrap()
            .uv
            .as_ref()
            .map(|details| details.kind),
        Some(UvPackageKind::Packaged)
    );

    let loader = TestTurboJsonLoader::new(root_turbo_jsons(json!({ "build": {} })));
    let engine = uv_engine(&repo_root, &package_graph, &loader, "build", Vec::new());

    let def = task_definition(&engine, "app#build");
    // The project's own sources are hashed by default, plus its transitive
    // dependency members (uv builds against their sources) and the
    // workspace-level files whose changes must invalidate the cache.
    assert!(def.inputs.default);
    for input in [
        "../../packages/lib-a/**",
        "!../../packages/lib-a/.venv/**",
        "../../pyproject.toml",
        "../../uv.toml",
        "../../.python-version",
    ] {
        assert!(
            def.inputs.globs.iter().any(|glob| glob == input),
            "missing input glob {input}, got {:?}",
            def.inputs.globs
        );
    }
    // Env vars that change what uv resolves are hashed.
    for var in ["UV_PYTHON", "UV_INDEX_URL", "UV_EXTRA_INDEX_URL"] {
        assert!(
            def.env.iter().any(|env| env == var),
            "missing env var {var}, got {:?}",
            def.env
        );
    }
    // Outputs are the project's wheel/sdist in the workspace root's dist/.
    assert_eq!(def.outputs.inclusions, vec!["../../dist/app-*"]);
}

#[test]
fn test_uv_virtual_tasks_stay_vanilla() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = uv_package_graph(&repo_root);

    let loader = TestTurboJsonLoader::new(root_turbo_jsons(json!({ "build": {} })));
    let engine = uv_engine(&repo_root, &package_graph, &loader, "build", Vec::new());

    // Virtual projects never execute (there is nothing for uv to build), so
    // their phantom tasks get no uv wiring.
    let def = task_definition(&engine, "virt#build");
    assert!(
        !def.inputs
            .globs
            .iter()
            .any(|glob| glob.contains("pyproject")),
        "virtual tasks should not hash workspace files, got {:?}",
        def.inputs.globs
    );
    assert!(
        def.outputs.inclusions.is_empty(),
        "virtual tasks have no outputs, got {:?}",
        def.outputs.inclusions
    );
    assert!(
        !def.env.iter().any(|env| env == "UV_PYTHON"),
        "virtual tasks should not hash uv env vars, got {:?}",
        def.env
    );
}

#[test]
fn test_uv_workspace_task_hashes_member_dirs_not_whole_repo() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = uv_package_graph(&repo_root);

    let loader = TestTurboJsonLoader::new(root_turbo_jsons(json!({ "sync": {} })));
    let engine = uv_engine(&repo_root, &package_graph, &loader, "sync", Vec::new());

    let def = task_definition(&engine, "uv#sync");
    // The workspace package's directory is the repo root; hashing by default
    // would pull the entire repository (including JS packages) into the
    // hash. Member directories are hashed instead.
    assert!(
        !def.inputs.default,
        "workspace tasks must not default-hash the repo root"
    );
    for input in [
        "packages/app/**",
        "packages/lib-a/**",
        "packages/virt/**",
        "!packages/app/.venv/**",
        "./pyproject.toml",
    ] {
        assert!(
            def.inputs.globs.iter().any(|glob| glob == input),
            "missing input glob {input}, got {:?}",
            def.inputs.globs
        );
    }
}

#[test]
fn test_uv_packaged_respects_explicit_inputs() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = uv_package_graph(&repo_root);

    let loader = TestTurboJsonLoader::new(root_turbo_jsons(
        json!({ "build": { "inputs": ["src/**"] } }),
    ));
    let engine = uv_engine(&repo_root, &package_graph, &loader, "build", Vec::new());

    let def = task_definition(&engine, "app#build");
    // A user's explicit `inputs` narrows hashing; the uv wiring must not
    // silently widen it again. Workspace-level globs are still appended so
    // manifest/interpreter changes invalidate the cache.
    assert!(
        !def.inputs.default,
        "explicit inputs config must not be overridden"
    );
    assert!(def.inputs.globs.iter().any(|glob| glob == "src/**"));
    assert!(
        !def.inputs
            .globs
            .iter()
            .any(|glob| glob == "../../packages/lib-a/**"),
        "explicit inputs must not be widened with dependency globs, got {:?}",
        def.inputs.globs
    );
    assert!(
        def.inputs
            .globs
            .iter()
            .any(|glob| glob == "../../pyproject.toml")
    );
}

#[test]
fn test_uv_packaged_turbo_default_keeps_automatic_inputs() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = uv_package_graph(&repo_root);

    let loader = TestTurboJsonLoader::new(root_turbo_jsons(
        json!({ "build": { "inputs": ["$TURBO_DEFAULT$", "$TURBO_ROOT$/version.txt"] } }),
    ));
    let engine = uv_engine(&repo_root, &package_graph, &loader, "build", Vec::new());

    let def = task_definition(&engine, "app#build");
    // `$TURBO_DEFAULT$` opts back into everything turbo hashes
    // automatically for the project — its own sources and the flattened
    // dependency closure — so extra inputs are additive.
    assert!(def.inputs.default);
    for input in [
        "../../version.txt",
        "../../packages/lib-a/**",
        "../../pyproject.toml",
    ] {
        assert!(
            def.inputs.globs.iter().any(|glob| glob == input),
            "missing input glob {input}, got {:?}",
            def.inputs.globs
        );
    }
}

#[test]
fn test_uv_tasks_receive_global_inputs() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = uv_package_graph(&repo_root);

    let loader = TestTurboJsonLoader::new(root_turbo_jsons(json!({ "build": {} })));
    let engine = uv_engine(
        &repo_root,
        &package_graph,
        &loader,
        "build",
        vec!["configs/**".to_string()],
    );

    // Packaged tasks execute, so they hash global inputs like script-backed
    // tasks do.
    let def = task_definition(&engine, "app#build");
    assert!(
        def.inputs
            .globs
            .iter()
            .any(|glob| glob == "../../configs/**"),
        "global inputs must apply to executing uv tasks, got {:?}",
        def.inputs.globs
    );
    // Virtual tasks are phantoms and must not hash global inputs (their
    // hashes would churn and cascade into dependents).
    let virt_def = task_definition(&engine, "virt#build");
    assert!(
        !virt_def
            .inputs
            .globs
            .iter()
            .any(|glob| glob == "../../configs/**"),
        "global inputs must not apply to phantom virtual tasks, got {:?}",
        virt_def.inputs.globs
    );
}
