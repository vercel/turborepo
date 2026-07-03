use turborepo_repository::cargo::{CargoPackageKind, WORKSPACE_PACKAGE_NAME};

use super::*;

/// Build a package graph over a real Cargo workspace fixture in a tempdir:
/// `app` (bin entrypoint) depends on `lib-a` (library). Discovery shells out
/// to `cargo metadata`, so the manifests must be valid.
fn cargo_package_graph(repo_root: &turbopath::AbsoluteSystemPath) -> PackageGraph {
    let write = |rel: &[&str], contents: &str| {
        let path = repo_root.join_components(rel);
        std::fs::create_dir_all(path.parent().unwrap().as_std_path()).unwrap();
        std::fs::write(path.as_std_path(), contents).unwrap();
    };
    write(
        &["Cargo.toml"],
        "[workspace]\nmembers = [\"crates/*\"]\nresolver = \"2\"\n",
    );
    write(
        &["crates", "lib-a", "Cargo.toml"],
        "[package]\nname = \"lib-a\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write(&["crates", "lib-a", "src", "lib.rs"], "");
    write(
        &["crates", "app", "Cargo.toml"],
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \
         \"2021\"\n\n[dependencies]\nlib-a = { path = \"../lib-a\" }\n",
    );
    write(&["crates", "app", "src", "main.rs"], "fn main() {}\n");

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(
        PackageGraph::builder(repo_root, PackageJson::default())
            .with_package_discovery(MockDiscovery)
            .with_lockfile(Some(Box::new(MockLockfile)))
            .with_package_jsons(Some(HashMap::new()))
            .with_cargo(true)
            .build(),
    )
    .unwrap()
}

fn cargo_engine(
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
fn test_cargo_entrypoint_build_task_wiring() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = cargo_package_graph(&repo_root);
    assert_eq!(
        package_graph
            .package_info(&PackageName::from("app"))
            .unwrap()
            .cargo
            .as_ref()
            .map(|details| details.kind),
        Some(CargoPackageKind::Entrypoint)
    );

    let loader = TestTurboJsonLoader::new(root_turbo_jsons(json!({ "build": {} })));
    let engine = cargo_engine(&repo_root, &package_graph, &loader, "build", Vec::new());

    let def = task_definition(&engine, "app#build");
    // The entrypoint's own sources are hashed by default, plus its
    // transitive dependency crates (cargo compiles them in this task) and
    // the workspace-level files whose changes must invalidate the cache.
    assert!(def.inputs.default);
    for input in [
        "../../crates/lib-a/**",
        "../../Cargo.toml",
        "../../.cargo/config.toml",
        "../../rust-toolchain.toml",
    ] {
        assert!(
            def.inputs.globs.iter().any(|glob| glob == input),
            "missing input glob {input}, got {:?}",
            def.inputs.globs
        );
    }
    // Env vars that change what cargo builds are hashed.
    for var in [
        "RUSTFLAGS",
        "RUSTC_WRAPPER",
        "CARGO_TARGET_DIR",
        "CARGO_BUILD_TARGET",
    ] {
        assert!(
            def.env.iter().any(|env| env == var),
            "missing env var {var}, got {:?}",
            def.env
        );
    }
    // Outputs are the crate's bin deliverables — nothing else from target/.
    assert!(
        def.outputs
            .inclusions
            .iter()
            .any(|glob| glob == "../../target/*/app"),
        "missing bin output glob, got {:?}",
        def.outputs.inclusions
    );
    assert!(
        !def.outputs
            .inclusions
            .iter()
            .any(|glob| glob.contains("lib") || glob.ends_with(".rlib")),
        "only bin deliverables should be cached, got {:?}",
        def.outputs.inclusions
    );
}

#[test]
fn test_cargo_library_tasks_stay_vanilla() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = cargo_package_graph(&repo_root);

    let loader = TestTurboJsonLoader::new(root_turbo_jsons(json!({ "build": {} })));
    let engine = cargo_engine(&repo_root, &package_graph, &loader, "build", Vec::new());

    // Library crates never execute (cargo builds them implicitly as part of
    // an entrypoint's closure), so their phantom tasks get no cargo wiring.
    let def = task_definition(&engine, "lib-a#build");
    assert!(
        !def.inputs.globs.iter().any(|glob| glob.contains("Cargo")),
        "library tasks should not hash workspace files, got {:?}",
        def.inputs.globs
    );
    assert!(
        def.outputs.inclusions.is_empty(),
        "library tasks have no outputs, got {:?}",
        def.outputs.inclusions
    );
    assert!(
        !def.env.iter().any(|env| env == "RUSTFLAGS"),
        "library tasks should not hash cargo env vars, got {:?}",
        def.env
    );
}

#[test]
fn test_cargo_workspace_task_hashes_crate_dirs_not_whole_repo() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = cargo_package_graph(&repo_root);

    let loader = TestTurboJsonLoader::new(root_turbo_jsons(json!({ "test": {} })));
    let engine = cargo_engine(&repo_root, &package_graph, &loader, "test", Vec::new());

    let def = task_definition(&engine, "cargo#test");
    // The workspace package's directory is the repo root; hashing by default
    // would pull the entire repository (including JS packages) into the
    // hash. Crate directories are hashed instead.
    assert!(
        !def.inputs.default,
        "workspace tasks must not default-hash the repo root"
    );
    for input in ["crates/app/**", "crates/lib-a/**", "./Cargo.toml"] {
        assert!(
            def.inputs.globs.iter().any(|glob| glob == input),
            "missing input glob {input}, got {:?}",
            def.inputs.globs
        );
    }
}

#[test]
fn test_cargo_entrypoint_respects_explicit_inputs() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = cargo_package_graph(&repo_root);

    let loader = TestTurboJsonLoader::new(root_turbo_jsons(
        json!({ "build": { "inputs": ["src/**"] } }),
    ));
    let engine = cargo_engine(&repo_root, &package_graph, &loader, "build", Vec::new());

    let def = task_definition(&engine, "app#build");
    // A user's explicit `inputs` narrows hashing; the cargo wiring must not
    // silently widen it again. Workspace-level globs are still appended so
    // lockfile/toolchain changes invalidate the cache.
    assert!(
        !def.inputs.default,
        "explicit inputs config must not be overridden"
    );
    assert!(def.inputs.globs.iter().any(|glob| glob == "src/**"));
    assert!(
        !def.inputs
            .globs
            .iter()
            .any(|glob| glob == "../../crates/lib-a/**"),
        "explicit inputs must not be widened with dependency globs, got {:?}",
        def.inputs.globs
    );
    assert!(
        def.inputs
            .globs
            .iter()
            .any(|glob| glob == "../../Cargo.toml")
    );
}

#[test]
fn test_cargo_entrypoint_turbo_default_keeps_automatic_inputs() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = cargo_package_graph(&repo_root);

    let loader = TestTurboJsonLoader::new(root_turbo_jsons(
        json!({ "build": { "inputs": ["$TURBO_DEFAULT$", "$TURBO_ROOT$/version.txt"] } }),
    ));
    let engine = cargo_engine(&repo_root, &package_graph, &loader, "build", Vec::new());

    let def = task_definition(&engine, "app#build");
    // `$TURBO_DEFAULT$` opts back into everything turbo hashes
    // automatically for the crate — its own sources and the flattened
    // dependency closure — so extra inputs (e.g. a file embedded via
    // `include_str!` from outside any crate directory) are additive.
    assert!(def.inputs.default);
    for input in [
        "../../version.txt",
        "../../crates/lib-a/**",
        "../../Cargo.toml",
    ] {
        assert!(
            def.inputs.globs.iter().any(|glob| glob == input),
            "missing input glob {input}, got {:?}",
            def.inputs.globs
        );
    }
}

#[test]
fn test_cargo_workspace_turbo_default_keeps_crate_globs() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = cargo_package_graph(&repo_root);

    let loader = TestTurboJsonLoader::new(root_turbo_jsons(
        json!({ "test": { "inputs": ["$TURBO_DEFAULT$", "testdata/**"] } }),
    ));
    let engine = cargo_engine(&repo_root, &package_graph, &loader, "test", Vec::new());

    let def = task_definition(&engine, "cargo#test");
    // For the workspace package, `$TURBO_DEFAULT$` resolves to the crate
    // directories — never to default-hashing the repo root, which would
    // pull every JS package into the hash.
    assert!(
        !def.inputs.default,
        "workspace tasks must not default-hash the repo root even with $TURBO_DEFAULT$"
    );
    for input in ["crates/app/**", "crates/lib-a/**", "testdata/**"] {
        assert!(
            def.inputs.globs.iter().any(|glob| glob == input),
            "missing input glob {input}, got {:?}",
            def.inputs.globs
        );
    }
}

#[test]
fn test_cargo_tasks_receive_global_inputs() {
    let repo_root_dir = TempDir::with_prefix("repo").unwrap();
    let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
    let package_graph = cargo_package_graph(&repo_root);

    let loader = TestTurboJsonLoader::new(root_turbo_jsons(json!({ "build": {} })));
    let engine = cargo_engine(
        &repo_root,
        &package_graph,
        &loader,
        "build",
        vec!["configs/**".to_string()],
    );

    // Entrypoint tasks execute, so they hash global inputs like
    // script-backed tasks do.
    let def = task_definition(&engine, "app#build");
    assert!(
        def.inputs
            .globs
            .iter()
            .any(|glob| glob == "../../configs/**"),
        "global inputs must apply to executing cargo tasks, got {:?}",
        def.inputs.globs
    );
    // Library tasks are phantoms and must not hash global inputs (their
    // hashes would churn and cascade into dependents).
    let lib_def = task_definition(&engine, "lib-a#build");
    assert!(
        !lib_def
            .inputs
            .globs
            .iter()
            .any(|glob| glob == "../../configs/**"),
        "global inputs must not apply to phantom library tasks, got {:?}",
        lib_def.inputs.globs
    );
}
