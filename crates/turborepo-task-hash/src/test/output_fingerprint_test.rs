use turborepo_engine::{Building, Engine};
use turborepo_types::TaskOutputs;

use super::*;

/// A monorepo with three packages: `lib` and `app` have a `build` script,
/// `ui` has none (its consumers compile its sources).
async fn graph_with_scripts(repo_root: &AbsoluteSystemPathBuf) -> PackageGraph {
    let root_json = json!({
        "name": "root",
        "packageManager": "npm@10.0.0",
        "workspaces": ["packages/*"]
    });
    repo_root
        .join_component("package.json")
        .create_with_contents(serde_json::to_string(&root_json).unwrap())
        .unwrap();
    for (name, manifest) in [
        (
            "lib",
            json!({ "name": "lib", "scripts": { "build": "build-lib" } }),
        ),
        (
            "app",
            json!({ "name": "app", "scripts": { "build": "build-app" } }),
        ),
        ("ui", json!({ "name": "ui" })),
    ] {
        let path = repo_root.join_components(&["packages", name, "package.json"]);
        path.ensure_dir().unwrap();
        path.create_with_contents(serde_json::to_string(&manifest).unwrap())
            .unwrap();
    }
    PackageGraph::builder(repo_root, PackageJson::from_value(root_json).unwrap())
        .build()
        .await
        .unwrap()
}

fn with_outputs(globs: &[&str]) -> TaskDefinition {
    TaskDefinition {
        outputs: TaskOutputs {
            inclusions: globs.iter().map(|glob| glob.to_string()).collect(),
            exclusions: Vec::new(),
        },
        ..Default::default()
    }
}

fn write(repo_root: &AbsoluteSystemPathBuf, path: &[&str], contents: &str) {
    let file = repo_root.join_components(path);
    file.ensure_dir().unwrap();
    file.create_with_contents(contents).unwrap();
}

fn lib_output_hashes(repo_root: &AbsoluteSystemPathBuf, graph: &PackageGraph) -> FileHashes {
    let task_id = TaskId::new("lib", "build").into_owned();
    let context = graph
        .package_task_context(&PackageName::from("lib"))
        .unwrap();
    output_file_hashes(
        &SCM::new(repo_root),
        repo_root,
        &task_id,
        &with_outputs(&["dist/**", ".turbo/**"]),
        &context,
    )
    .unwrap()
}

/// A hasher that knows `app#build`'s file inputs and the given task hashes
/// of already-finished tasks.
fn hasher_with_finished<'a>(
    repo_root: &'a AbsoluteSystemPathBuf,
    run_opts: &'a TestRunOpts,
    env: &'a EnvironmentVariableMap,
    finished: &[(&TaskId<'static>, &str)],
) -> TaskHasher<'a, TestRunOpts> {
    let consumer = TaskId::new("app", "build").into_owned();
    let inputs = Arc::new(FileHashes(Vec::new()));
    let hasher = TaskHasher::new(
        PackageInputsHashes {
            hashes: HashMap::from([(consumer.clone(), inputs.as_ref().clone().hash())]),
            expanded_hashes: HashMap::from([(consumer, inputs)]),
        },
        run_opts,
        env,
        "global-hash",
        repo_root,
        EnvironmentVariableMap::default(),
        &[],
    );
    let tracker = hasher.task_hash_tracker();
    for (task_id, hash) in finished {
        tracker.insert_hash(
            (*task_id).clone(),
            DetailedMap::default(),
            Arc::from(*hash),
            None,
        );
    }
    hasher
}

fn app_hash(
    hasher: &TaskHasher<'_, TestRunOpts>,
    graph: &PackageGraph,
    dependencies: &[TaskId<'static>],
    fingerprints: Option<&OutputFingerprints>,
) -> String {
    let consumer = TaskId::new("app", "build").into_owned();
    let nodes = dependencies
        .iter()
        .cloned()
        .map(TaskNode::Task)
        .collect::<Vec<_>>();
    let dependency_set = nodes.iter().collect::<Vec<_>>();
    let context = graph
        .package_task_context(&PackageName::from("app"))
        .unwrap();
    let definition = TaskDefinition::default();
    let request = TaskHashRequest {
        task_id: &consumer,
        task_definition: &definition,
        task_env_mode: EnvMode::Strict,
        package_context: &context,
        dependency_set: &dependency_set,
        telemetry: PackageTaskEventBuilder::new("app", "build"),
    };
    match fingerprints {
        Some(fingerprints) => hasher
            .calculate_task_hash_with_output_fingerprints(request, fingerprints)
            .unwrap(),
        None => hasher
            .calculate_task_hash(
                &consumer,
                &definition,
                EnvMode::Strict,
                &context,
                &dependency_set,
                PackageTaskEventBuilder::new("app", "build"),
            )
            .unwrap(),
    }
}

fn lib_fingerprint(
    hasher: &TaskHasher<'_, TestRunOpts>,
    output_hashes: &FileHashes,
    dependencies: &[TaskId<'static>],
    dependency_fingerprints: &OutputFingerprints,
) -> Arc<str> {
    let nodes = dependencies
        .iter()
        .cloned()
        .map(TaskNode::Task)
        .collect::<Vec<_>>();
    let dependency_set = nodes.iter().collect::<Vec<_>>();
    hasher
        .output_fingerprint(
            &TaskId::new("lib", "build").into_owned(),
            output_hashes,
            &dependency_set,
            dependency_fingerprints,
        )
        .unwrap()
}

#[tokio::test]
async fn output_file_hashes_are_repo_relative_and_exclude_the_log_file() {
    let tmp = tempdir().unwrap();
    let repo_root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
    let graph = graph_with_scripts(&repo_root).await;
    write(&repo_root, &["packages", "lib", "dist", "index.js"], "v1\n");
    write(&repo_root, &["packages", "lib", "README.md"], "docs\n");
    write(
        &repo_root,
        &["packages", "lib", ".turbo", "turbo-build.log"],
        "build took 12ms\n",
    );
    write(
        &repo_root,
        &["packages", "lib", ".turbo", "other.txt"],
        "x\n",
    );

    let hashes = lib_output_hashes(&repo_root, &graph);
    let paths = hashes
        .0
        .iter()
        .map(|(path, _)| path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        [
            "packages/lib/.turbo/other.txt",
            "packages/lib/dist/index.js",
            "packages/lib/package.json",
        ],
        "outputs and package configuration are hashed; the log file and other inputs are not"
    );
}

#[tokio::test]
async fn output_file_hashes_are_identical_across_checkouts() {
    let mut hashes = Vec::new();
    for _ in 0..2 {
        let tmp = tempdir().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
        let graph = graph_with_scripts(&repo_root).await;
        write(&repo_root, &["packages", "lib", "dist", "index.js"], "v1\n");
        hashes.push(lib_output_hashes(&repo_root, &graph));
    }
    let entries = |hashes: &FileHashes| {
        hashes
            .0
            .iter()
            .map(|(path, hash)| (path.to_string(), hash.to_string()))
            .collect::<Vec<_>>()
    };
    assert_eq!(entries(&hashes[0]), entries(&hashes[1]));
}

#[tokio::test]
async fn fingerprint_replaces_dependency_task_hash() {
    let tmp = tempdir().unwrap();
    let repo_root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
    let graph = graph_with_scripts(&repo_root).await;
    let lib = TaskId::new("lib", "build").into_owned();
    let run_opts = TestRunOpts {
        single_package: true,
    };
    let env = EnvironmentVariableMap::default();
    write(&repo_root, &["packages", "lib", "dist", "index.js"], "v1\n");
    let outputs_v1 = lib_output_hashes(&repo_root, &graph);

    let mut hashes = Vec::new();
    for lib_task_hash in ["lib-task-hash-v1", "lib-task-hash-v2"] {
        let hasher = hasher_with_finished(&repo_root, &run_opts, &env, &[(&lib, lib_task_hash)]);
        let fingerprint = lib_fingerprint(&hasher, &outputs_v1, &[], &OutputFingerprints::new());
        let fingerprints = OutputFingerprints::from([(lib.clone(), fingerprint)]);
        hashes.push(app_hash(
            &hasher,
            &graph,
            std::slice::from_ref(&lib),
            Some(&fingerprints),
        ));
    }
    assert_eq!(
        hashes[0], hashes[1],
        "a rerun dependency with identical outputs keeps its dependent's hash"
    );

    write(&repo_root, &["packages", "lib", "dist", "index.js"], "v2\n");
    let outputs_v2 = lib_output_hashes(&repo_root, &graph);
    let hasher = hasher_with_finished(&repo_root, &run_opts, &env, &[(&lib, "lib-task-hash-v1")]);
    let fingerprint = lib_fingerprint(&hasher, &outputs_v2, &[], &OutputFingerprints::new());
    let fingerprints = OutputFingerprints::from([(lib.clone(), fingerprint)]);
    assert_ne!(
        hashes[0],
        app_hash(
            &hasher,
            &graph,
            std::slice::from_ref(&lib),
            Some(&fingerprints)
        ),
        "changed dependency outputs change the dependent's hash"
    );
}

#[tokio::test]
async fn empty_fingerprints_keep_the_task_hash() {
    let tmp = tempdir().unwrap();
    let repo_root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
    let graph = graph_with_scripts(&repo_root).await;
    let lib = TaskId::new("lib", "build").into_owned();
    let run_opts = TestRunOpts {
        single_package: true,
    };
    let env = EnvironmentVariableMap::default();
    let hasher = hasher_with_finished(&repo_root, &run_opts, &env, &[(&lib, "lib-task-hash")]);
    let dependencies = std::slice::from_ref(&lib);

    assert_eq!(
        app_hash(&hasher, &graph, dependencies, None),
        app_hash(
            &hasher,
            &graph,
            dependencies,
            Some(&OutputFingerprints::new())
        ),
    );
}

#[tokio::test]
async fn fingerprint_covers_dependencies_of_the_dependency() {
    let tmp = tempdir().unwrap();
    let repo_root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
    let graph = graph_with_scripts(&repo_root).await;
    let transit = TaskId::new("ui", "transit").into_owned();
    let codegen = TaskId::new("ui", "codegen").into_owned();
    let run_opts = TestRunOpts {
        single_package: true,
    };
    let env = EnvironmentVariableMap::default();
    write(&repo_root, &["packages", "lib", "dist", "index.js"], "v1\n");
    let outputs = lib_output_hashes(&repo_root, &graph);

    let fingerprint_with = |transit_hash: &str, codegen_fingerprint: &str| {
        let hasher = hasher_with_finished(
            &repo_root,
            &run_opts,
            &env,
            &[(&transit, transit_hash), (&codegen, "codegen-task-hash")],
        );
        lib_fingerprint(
            &hasher,
            &outputs,
            &[transit.clone(), codegen.clone()],
            &OutputFingerprints::from([(codegen.clone(), Arc::from(codegen_fingerprint))]),
        )
    };

    let base = fingerprint_with("transit-v1", "codegen-fingerprint-v1");
    assert_eq!(
        base,
        fingerprint_with("transit-v1", "codegen-fingerprint-v1"),
        "fingerprints are deterministic"
    );
    assert_ne!(
        base,
        fingerprint_with("transit-v2", "codegen-fingerprint-v1"),
        "a dependency without outputs contributes its task hash"
    );
    assert_ne!(
        base,
        fingerprint_with("transit-v1", "codegen-fingerprint-v2"),
        "a dependency with outputs contributes its fingerprint"
    );
}

#[tokio::test]
async fn fingerprint_covers_external_dependencies() {
    let tmp = tempdir().unwrap();
    let repo_root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
    let graph = graph_with_scripts(&repo_root).await;
    let run_opts = TestRunOpts {
        single_package: false,
    };
    let env = EnvironmentVariableMap::default();
    write(&repo_root, &["packages", "lib", "dist", "index.js"], "v1\n");
    let outputs = lib_output_hashes(&repo_root, &graph);

    let fingerprint_with = |external_deps_hash: &str| {
        let mut hasher = hasher_with_finished(&repo_root, &run_opts, &env, &[]);
        hasher.set_external_deps_hash_cache(HashMap::from([(
            "lib".to_string(),
            external_deps_hash.to_string(),
        )]));
        lib_fingerprint(&hasher, &outputs, &[], &OutputFingerprints::new())
    };

    assert_ne!(
        fingerprint_with("lockfile-v1"),
        fingerprint_with("lockfile-v2")
    );
}

#[tokio::test]
async fn only_dependencies_that_run_a_command_with_outputs_contribute_fingerprints() {
    let tmp = tempdir().unwrap();
    let repo_root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
    let graph = graph_with_scripts(&repo_root).await;
    let lib_build = TaskId::new("lib", "build").into_owned();
    let lib_lint = TaskId::new("lib", "lint").into_owned();
    let ui_build = TaskId::new("ui", "build").into_owned();
    let app_build = TaskId::new("app", "build").into_owned();

    assert!(contributes_output_fingerprint(
        &lib_build,
        &with_outputs(&["dist/**"]),
        &graph
    ));
    assert!(
        !contributes_output_fingerprint(&lib_build, &TaskDefinition::default(), &graph),
        "no outputs"
    );
    assert!(
        !contributes_output_fingerprint(&ui_build, &with_outputs(&["dist/**"]), &graph),
        "no command: consumers may compile the package's sources"
    );
    assert!(
        !contributes_output_fingerprint(
            &lib_build,
            &TaskDefinition {
                command: Some(TaskCommandOverride::OptOut),
                ..with_outputs(&["dist/**"])
            },
            &graph
        ),
        "command opted out"
    );
    assert!(
        contributes_output_fingerprint(
            &lib_lint,
            &TaskDefinition {
                command: Some(TaskCommandOverride::Argv(vec!["lint".to_string()])),
                ..with_outputs(&["report/**"])
            },
            &graph
        ),
        "command override"
    );

    let mut engine: Engine<Building, TaskDefinition> = Engine::new();
    for (task_id, definition) in [
        (&lib_build, with_outputs(&["dist/**"])),
        (&lib_lint, TaskDefinition::default()),
        (&ui_build, with_outputs(&["dist/**"])),
        (&app_build, with_outputs(&["dist/**"])),
    ] {
        engine.get_index(task_id);
        engine.add_definition(task_id.clone(), definition);
    }
    let app_index = engine.get_index(&app_build);
    for dependency in [&lib_build, &lib_lint, &ui_build] {
        let index = engine.get_index(dependency);
        engine.task_graph_mut().add_edge(app_index, index, ());
    }
    let engine = engine.seal();

    assert_eq!(
        output_fingerprint_dependencies(&engine, &app_build, &graph),
        std::slice::from_ref(&lib_build)
    );
    assert!(output_fingerprint_dependencies(&engine, &lib_build, &graph).is_empty());

    // Explicit `dependencyOutputs` inputs keep their own semantics.
    let mut engine: Engine<Building, TaskDefinition> = Engine::new();
    let explicit = TaskDefinition {
        inputs: TaskInputs {
            dependency_outputs: Some(turborepo_types::DependencyOutputsInput {
                from: None,
                globs: Vec::new(),
            }),
            ..Default::default()
        },
        ..with_outputs(&["dist/**"])
    };
    for (task_id, definition) in [
        (&lib_build, with_outputs(&["dist/**"])),
        (&app_build, explicit),
    ] {
        engine.get_index(task_id);
        engine.add_definition(task_id.clone(), definition);
    }
    let app_index = engine.get_index(&app_build);
    let lib_index = engine.get_index(&lib_build);
    engine.task_graph_mut().add_edge(app_index, lib_index, ());
    let engine = engine.seal();
    assert!(output_fingerprint_dependencies(&engine, &app_build, &graph).is_empty());
}
