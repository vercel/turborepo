use super::*;

fn task_hasher_with_file_inputs<'a>(
    repo_root: &'a AbsoluteSystemPathBuf,
    run_opts: &'a TestRunOpts,
    env: &'a EnvironmentVariableMap,
    file_inputs: HashMap<TaskId<'static>, Arc<FileHashes>>,
) -> TaskHasher<'a, TestRunOpts> {
    let hashes = file_inputs
        .iter()
        .map(|(task_id, file_hashes)| (task_id.clone(), file_hashes.as_ref().clone().hash()))
        .collect();
    TaskHasher::new(
        PackageInputsHashes {
            hashes,
            expanded_hashes: file_inputs,
        },
        run_opts,
        env,
        "global-hash",
        repo_root,
        EnvironmentVariableMap::default(),
        &[],
    )
}

fn jit_task_definition(globs: &[&str]) -> TaskDefinition {
    TaskDefinition {
        inputs: TaskInputs {
            jit_globs: globs.iter().map(|glob| glob.to_string()).collect(),
            eager: false,
            ..TaskInputs::default()
        },
        ..TaskDefinition::default()
    }
}

fn calculate_deferred_task_hash(
    hasher: &TaskHasher<'_, TestRunOpts>,
    repo_root: &AbsoluteSystemPathBuf,
    graph: &PackageGraph,
    task_id: &TaskId<'static>,
    definition: &TaskDefinition,
) -> String {
    let package = PackageName::from(task_id.package());
    hasher
        .calculate_task_hash_with_deferred_inputs(
            task_id,
            definition,
            EnvMode::Strict,
            &graph.package_task_context(&package).unwrap(),
            &[],
            PackageTaskEventBuilder::new(task_id.package(), task_id.task()),
            &SCM::new(repo_root),
            repo_root,
            None,
            None,
            &HashSet::new(),
        )
        .unwrap()
}

fn write_file(repo_root: &AbsoluteSystemPathBuf, package: &str, path: &str, contents: &str) {
    let mut components = vec!["packages", package];
    components.extend(path.split('/'));
    let absolute_path = repo_root.join_components(&components);
    absolute_path.ensure_dir().unwrap();
    absolute_path.create_with_contents(contents).unwrap();
}

fn empty_eager_inputs() -> Arc<FileHashes> {
    Arc::new(FileHashes(Vec::new()))
}

#[tokio::test]
async fn jit_inputs_are_hashed_after_dependency_files_are_materialized() {
    let tmp = tempdir().unwrap();
    let repo_root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
    let graph = javascript_graph(&repo_root).await;
    let task_id = TaskId::new("app", "build").into_owned();
    let definition = jit_task_definition(&["src/generated/**"]);
    let run_opts = TestRunOpts {
        single_package: true,
    };
    let env = EnvironmentVariableMap::default();

    write_file(&repo_root, "app", "src/generated/schema.txt", "schema-v1\n");
    let first_hasher = task_hasher_with_file_inputs(
        &repo_root,
        &run_opts,
        &env,
        HashMap::from([(task_id.clone(), empty_eager_inputs())]),
    );
    first_hasher
        .insert_deferred_hash(
            &task_id,
            &definition,
            EnvMode::Strict,
            &graph
                .package_task_context(&PackageName::from("app"))
                .unwrap(),
        )
        .unwrap();
    assert_eq!(
        first_hasher.task_hash_tracker().hash(&task_id).as_deref(),
        Some(JIT_DEFERRED_TASK_HASH_MESSAGE)
    );
    let first =
        calculate_deferred_task_hash(&first_hasher, &repo_root, &graph, &task_id, &definition);

    // This file represents output produced by a dependency before the task's
    // deferred JIT inputs are hashed.
    write_file(&repo_root, "app", "src/generated/schema.txt", "schema-v2\n");
    let second_hasher = task_hasher_with_file_inputs(
        &repo_root,
        &run_opts,
        &env,
        HashMap::from([(task_id.clone(), empty_eager_inputs())]),
    );
    let second =
        calculate_deferred_task_hash(&second_hasher, &repo_root, &graph, &task_id, &definition);

    assert_ne!(
        first, second,
        "JIT output changes must change the task hash"
    );
}

fn hash_jit_dependency_and_startup_descendant(
    repo_root: &AbsoluteSystemPathBuf,
    graph: &PackageGraph,
    generate: &TaskId<'static>,
    generate_definition: &TaskDefinition,
    build: &TaskId<'static>,
    build_definition: &TaskDefinition,
    build_inputs: Arc<FileHashes>,
) -> (String, String) {
    let run_opts = TestRunOpts {
        single_package: true,
    };
    let env = EnvironmentVariableMap::default();
    let hasher = task_hasher_with_file_inputs(
        repo_root,
        &run_opts,
        &env,
        HashMap::from([
            (generate.clone(), empty_eager_inputs()),
            (build.clone(), build_inputs),
        ]),
    );
    let generate_context = graph
        .package_task_context(&PackageName::from(generate.package()))
        .unwrap();
    hasher
        .insert_deferred_hash(
            generate,
            generate_definition,
            EnvMode::Strict,
            &generate_context,
        )
        .unwrap();
    let generate_hash =
        calculate_deferred_task_hash(&hasher, repo_root, graph, generate, generate_definition);

    let build_context = graph
        .package_task_context(&PackageName::from(build.package()))
        .unwrap();
    let generate_node = TaskNode::Task(generate.clone());
    let build_hash = hasher
        .calculate_task_hash(
            build,
            build_definition,
            EnvMode::Strict,
            &build_context,
            &[&generate_node],
            PackageTaskEventBuilder::new(build.package(), build.task()),
        )
        .unwrap();
    (generate_hash, build_hash)
}

#[tokio::test]
async fn descendants_hash_with_resolved_jit_dependency_and_pre_dependency_startup_inputs() {
    let tmp = tempdir().unwrap();
    let repo_root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
    let graph = javascript_graph(&repo_root).await;
    let generate = TaskId::new("app", "generate").into_owned();
    let build = TaskId::new("app", "build").into_owned();
    let generate_definition = jit_task_definition(&["jit-input.txt"]);
    let build_definition = TaskDefinition {
        inputs: TaskInputs::new(vec!["marker.txt".to_string()]),
        ..TaskDefinition::default()
    };

    write_file(&repo_root, "app", "marker.txt", "before\n");
    write_file(&repo_root, "app", "jit-input.txt", "stable\n");
    let build_context = graph
        .package_task_context(&PackageName::from("app"))
        .unwrap();
    let first_build_inputs = file_hashes_for_inputs(
        &SCM::new(&repo_root),
        &repo_root,
        build_context.directory(),
        &["marker.txt"],
        false,
        None,
    )
    .unwrap();

    // Simulate the dependency finishing after startup inputs were snapshotted.
    write_file(&repo_root, "app", ".generated/done.txt", "done\n");
    write_file(&repo_root, "app", "marker.txt", "after\n");
    let first = hash_jit_dependency_and_startup_descendant(
        &repo_root,
        &graph,
        &generate,
        &generate_definition,
        &build,
        &build_definition,
        first_build_inputs,
    );

    // A later run observes the same startup marker before the dependency
    // executes. The dependency may rewrite it again, but the descendant's
    // hash uses the startup snapshot and the resolved JIT dependency hash.
    write_file(&repo_root, "app", "marker.txt", "before\n");
    let second_build_inputs = file_hashes_for_inputs(
        &SCM::new(&repo_root),
        &repo_root,
        build_context.directory(),
        &["marker.txt"],
        false,
        None,
    )
    .unwrap();
    write_file(&repo_root, "app", ".generated/done.txt", "done\n");
    write_file(&repo_root, "app", "marker.txt", "after\n");
    let second = hash_jit_dependency_and_startup_descendant(
        &repo_root,
        &graph,
        &generate,
        &generate_definition,
        &build,
        &build_definition,
        second_build_inputs,
    );

    assert_eq!(first.0, second.0, "stable JIT inputs produce a stable hash");
    assert_eq!(
        first.1, second.1,
        "the descendant uses the startup snapshot and resolved JIT hash"
    );
}
