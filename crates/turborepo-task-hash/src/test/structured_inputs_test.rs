use super::*;

fn task_definition_with_inputs(inputs: TaskInputs) -> TaskDefinition {
    TaskDefinition {
        inputs,
        ..TaskDefinition::default()
    }
}

fn package_inputs_hashes(
    repo_root: &AbsoluteSystemPathBuf,
    graph: &PackageGraph,
    task_id: &TaskId<'static>,
    definition: &TaskDefinition,
) -> PackageInputsHashes {
    let tasks = [TaskNode::Task(task_id.clone())];
    let definitions = HashMap::from([(task_id.clone(), definition.clone())]);
    PackageInputsHashes::calculate_file_hashes(
        &SCM::new(repo_root),
        tasks.iter(),
        graph,
        &definitions,
        repo_root,
        &GenericEventBuilder::new(),
        None,
        true,
    )
    .unwrap()
}

fn task_hasher_for_inputs<'a>(
    repo_root: &'a AbsoluteSystemPathBuf,
    run_opts: &'a TestRunOpts,
    env: &'a EnvironmentVariableMap,
    package_inputs: PackageInputsHashes,
) -> TaskHasher<'a, TestRunOpts> {
    TaskHasher::new(
        package_inputs,
        run_opts,
        env,
        "global-hash",
        repo_root,
        EnvironmentVariableMap::default(),
        &[],
    )
}

fn eager_task_hash(
    repo_root: &AbsoluteSystemPathBuf,
    graph: &PackageGraph,
    task_id: &TaskId<'static>,
    definition: &TaskDefinition,
) -> String {
    let run_opts = TestRunOpts {
        single_package: true,
    };
    let env = EnvironmentVariableMap::default();
    let hasher = task_hasher_for_inputs(
        repo_root,
        &run_opts,
        &env,
        package_inputs_hashes(repo_root, graph, task_id, definition),
    );
    let package = PackageName::from(task_id.package());
    hasher
        .calculate_task_hash(
            task_id,
            definition,
            EnvMode::Strict,
            &graph.package_task_context(&package).unwrap(),
            &[],
            PackageTaskEventBuilder::new(task_id.package(), task_id.task()),
        )
        .unwrap()
}

fn deferred_task_hash(
    repo_root: &AbsoluteSystemPathBuf,
    graph: &PackageGraph,
    task_id: &TaskId<'static>,
    definition: &TaskDefinition,
) -> String {
    let run_opts = TestRunOpts {
        single_package: true,
    };
    let env = EnvironmentVariableMap::default();
    let hasher = task_hasher_for_inputs(
        repo_root,
        &run_opts,
        &env,
        package_inputs_hashes(repo_root, graph, task_id, definition),
    );
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

fn write_app_file(repo_root: &AbsoluteSystemPathBuf, path: &str, contents: &str) {
    let mut components = vec!["packages", "app"];
    components.extend(path.split('/'));
    let absolute_path = repo_root.join_components(&components);
    absolute_path.ensure_dir().unwrap();
    absolute_path.create_with_contents(contents).unwrap();
}

fn write_root_file(repo_root: &AbsoluteSystemPathBuf, name: &str, contents: &str) {
    let path = repo_root.join_component(name);
    path.ensure_dir().unwrap();
    path.create_with_contents(contents).unwrap();
}

#[tokio::test]
async fn structured_startup_defaults_include_files_except_exclusions() {
    let tmp = tempdir().unwrap();
    let repo_root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
    let graph = javascript_graph(&repo_root).await;
    let task_id = TaskId::new("app", "build").into_owned();
    let definition = task_definition_with_inputs(TaskInputs {
        globs: vec!["!excluded.txt".to_string(), "!.output/**".to_string()],
        default: true,
        ..TaskInputs::default()
    });
    write_app_file(&repo_root, "included.txt", "included-v1\n");
    write_app_file(&repo_root, "excluded.txt", "excluded-v1\n");
    write_app_file(&repo_root, ".output/cached.txt", "cached-v1\n");

    let baseline = eager_task_hash(&repo_root, &graph, &task_id, &definition);
    write_app_file(&repo_root, "excluded.txt", "excluded-v2\n");
    write_app_file(&repo_root, ".output/cached.txt", "cached-v2\n");
    assert_eq!(
        baseline,
        eager_task_hash(&repo_root, &graph, &task_id, &definition),
        "excluded and output files must not affect startup-default hashing"
    );

    write_app_file(&repo_root, "included.txt", "included-v2\n");
    assert_ne!(
        baseline,
        eager_task_hash(&repo_root, &graph, &task_id, &definition),
        "an included package file must affect startup-default hashing"
    );
}

#[tokio::test]
async fn structured_jit_defaults_use_deferred_package_inputs() {
    let tmp = tempdir().unwrap();
    let repo_root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
    let graph = javascript_graph(&repo_root).await;
    let task_id = TaskId::new("app", "build").into_owned();
    let definition = task_definition_with_inputs(TaskInputs {
        jit_globs: vec!["!excluded.txt".to_string(), "!.output/**".to_string()],
        jit_default: true,
        eager: false,
        ..TaskInputs::default()
    });
    write_app_file(&repo_root, "included.txt", "included-v1\n");
    write_app_file(&repo_root, "excluded.txt", "excluded-v1\n");
    write_app_file(&repo_root, ".output/cached.txt", "cached-v1\n");

    let baseline = deferred_task_hash(&repo_root, &graph, &task_id, &definition);
    write_app_file(&repo_root, "excluded.txt", "excluded-v2\n");
    write_app_file(&repo_root, ".output/cached.txt", "cached-v2\n");
    assert_eq!(
        baseline,
        deferred_task_hash(&repo_root, &graph, &task_id, &definition),
        "excluded and output files must not affect deferred JIT defaults"
    );

    write_app_file(&repo_root, "included.txt", "included-v2\n");
    assert_ne!(
        baseline,
        deferred_task_hash(&repo_root, &graph, &task_id, &definition),
        "an included package file must affect deferred JIT-default hashing"
    );
}

#[tokio::test]
async fn startup_inputs_hash_turbo_root_globs_from_package_scope() {
    let tmp = tempdir().unwrap();
    let repo_root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
    let graph = javascript_graph(&repo_root).await;
    let task_id = TaskId::new("app", "build").into_owned();
    let definition = task_definition_with_inputs(TaskInputs::new(vec![
        "../../root-config.txt".to_string(),
        "!.output/**".to_string(),
    ]));
    write_root_file(&repo_root, "root-config.txt", "root-v1\n");
    write_app_file(&repo_root, "local.txt", "local-v1\n");

    let baseline = eager_task_hash(&repo_root, &graph, &task_id, &definition);
    write_root_file(&repo_root, "root-config.txt", "root-v2\n");
    assert_ne!(
        baseline,
        eager_task_hash(&repo_root, &graph, &task_id, &definition),
        "the normalized package-relative root glob must hash the root file"
    );
}

#[tokio::test]
async fn jit_inputs_hash_turbo_root_globs_after_dependencies_complete() {
    let tmp = tempdir().unwrap();
    let repo_root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
    let graph = javascript_graph(&repo_root).await;
    let task_id = TaskId::new("app", "build").into_owned();
    let definition = task_definition_with_inputs(TaskInputs {
        jit_globs: vec!["../../root-generated.txt".to_string()],
        eager: false,
        ..TaskInputs::default()
    });
    write_root_file(&repo_root, "root-generated.txt", "root-generated-v1\n");

    let baseline = deferred_task_hash(&repo_root, &graph, &task_id, &definition);
    write_root_file(&repo_root, "root-generated.txt", "root-generated-v2\n");
    assert_ne!(
        baseline,
        deferred_task_hash(&repo_root, &graph, &task_id, &definition),
        "the normalized package-relative JIT root glob must hash the root file"
    );
}
